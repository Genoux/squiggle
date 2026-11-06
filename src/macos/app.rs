//! The menu bar app: the controller AppKit calls into, the status menu, and
//! the dialogs. The run loop itself is `surface::Engine`.

use crate::macos::ax;
use crate::macos::dictionary::Dictionary;
use crate::macos::host::MacHost;
use crate::macos::overlay::Overlay;
use crate::surface::menu::{Command, Item, menu};
use crate::surface::{
    ADD_CREDITS, Action, Engine, Status, TICK, parse_key, recent_log, update_config,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationPolicy, NSImage,
    NSMenu, NSMenuItem, NSSecureTextField, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
    NSWorkspace, NSWorkspaceDidActivateApplicationNotification,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSString, NSTimer, NSURL};
use squiggle_core::Config;
use std::cell::RefCell;

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct State {
    engine: Engine<MacHost, Overlay>,
    /// What the menu was last drawn from.
    drawn: Option<Status>,
    status: Option<Retained<NSStatusItem>>,
    menu: Option<Retained<NSMenu>>,
    /// What each drawn item's tag indexes, so one selector serves them all.
    commands: Vec<Command>,
    /// Built the first time it is opened, then kept.
    dictionary: Option<Dictionary>,
}

pub struct Ivars {
    state: RefCell<Option<State>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SquiggleController"]
    #[ivars = Ivars]
    pub struct Controller;

    unsafe impl NSObjectProtocol for Controller {}

    impl Controller {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &AnyObject) {
            self.with_state(|state, me| {
                state.engine.tick();
                sync_menu(state, me);
            });
        }

        #[unsafe(method(accept:))]
        fn accept(&self, sender: &AnyObject) {
            self.act(Action::Accept, sender);
        }

        #[unsafe(method(dismiss:))]
        fn dismiss(&self, sender: &AnyObject) {
            self.act(Action::Dismiss, sender);
        }

        #[unsafe(method(learn:))]
        fn learn(&self, sender: &AnyObject) {
            let tagged = tag(sender);
            self.with_state(|state, me| {
                state.engine.act(Action::Learn, tagged);
                if let Some(dictionary) = state.dictionary.as_mut().filter(|d| d.visible()) {
                    dictionary.reload(me.target(), me.mtm());
                }
            });
        }

        #[unsafe(method(acceptSentence:))]
        fn accept_sentence(&self, sender: &AnyObject) {
            self.act(Action::AcceptSentence, sender);
        }

        #[unsafe(method(dismissSentence:))]
        fn dismiss_sentence(&self, sender: &AnyObject) {
            self.act(Action::DismissSentence, sender);
        }

        #[unsafe(method(more:))]
        fn more(&self, sender: &AnyObject) {
            self.act(Action::More, sender);
        }

        #[unsafe(method(offHere:))]
        fn off_here(&self, sender: &AnyObject) {
            self.act(Action::OffHere, sender);
        }

        #[unsafe(method(pauseHour:))]
        fn pause_hour(&self, sender: &AnyObject) {
            self.act(Action::PauseHour, sender);
        }

        #[unsafe(method(choose:))]
        fn choose(&self, sender: &AnyObject) {
            let index = tag(sender);
            let mut command = None;
            self.with_state(|state, _| command = state.commands.get(index).copied());
            if let Some(command) = command {
                self.run(command);
            }
        }

        #[unsafe(method(addWord:))]
        fn add_word(&self, _sender: &AnyObject) {
            self.with_state(|state, me| {
                let changed = state
                    .dictionary
                    .as_mut()
                    .is_some_and(|d| d.add(me.target(), me.mtm()));
                if changed {
                    state.engine.recheck();
                }
            });
        }

        #[unsafe(method(removeWord:))]
        fn remove_word(&self, sender: &AnyObject) {
            let index = tag(sender);
            self.with_state(|state, me| {
                let changed = state
                    .dictionary
                    .as_mut()
                    .is_some_and(|d| d.remove(index, me.target(), me.mtm()));
                if changed {
                    state.engine.recheck();
                }
            });
        }

    }
);

fn tag(sender: &AnyObject) -> usize {
    let tag: isize = unsafe { msg_send![sender, tag] };
    tag.max(0) as usize
}

impl Controller {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            state: RefCell::new(None),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn with_state(&self, f: impl FnOnce(&mut State, &Self)) {
        // A re-entrant call (AppKit calling back into an action while another
        // holds the state) is dropped rather than panicking the app; the next
        // tick sees the same world again.
        let Ok(mut guard) = self.ivars().state.try_borrow_mut() else {
            return;
        };
        if let Some(state) = guard.as_mut() {
            f(state, self);
        }
    }

    fn act(&self, action: Action, sender: &AnyObject) {
        let tagged = tag(sender);
        self.with_state(|state, _| state.engine.act(action, tagged));
    }

    fn run(&self, command: Command) {
        match command {
            Command::PauseFor(index) => self.update(|engine| engine.pause_for(index)),
            Command::Resume => self.update(Engine::resume),
            Command::TurnOn(index) => self.update(|engine| engine.turn_on(index)),
            Command::TurnAllOn => self.update(Engine::turn_all_on),
            Command::AddCredits => {
                if let Some(url) = NSURL::URLWithString(&NSString::from_str(ADD_CREDITS)) {
                    NSWorkspace::sharedWorkspace().openURL(&url);
                }
            }
            Command::ToggleAi => self.update(Engine::toggle_ai),
            Command::SetKey => {
                ask_for_key(self.mtm());
                self.update(|engine| {
                    engine.clear_ai_error();
                    engine.reset();
                });
            }
            Command::Spelling(index) => self.update(|engine| engine.set_dialect(index)),
            Command::Dictionary => self.with_state(|state, me| {
                let (target, mtm) = (me.target(), me.mtm());
                state
                    .dictionary
                    .get_or_insert_with(|| Dictionary::new(target, mtm))
                    .show(target, mtm);
            }),
            Command::CopyLog => copy_recent_log(self.mtm()),
            Command::GrantAccess => grant_access(),
            Command::Quit => NSApplication::sharedApplication(self.mtm()).terminate(None),
        }
    }

    fn update(&self, change: impl FnOnce(&mut Engine<MacHost, Overlay>)) {
        self.with_state(|state, me| {
            change(&mut state.engine);
            sync_menu(state, me);
        });
    }

    fn target(&self) -> &AnyObject {
        let object: &NSObject = self.as_super();
        object
    }

    fn handle(&self) -> Retained<AnyObject> {
        let object: Retained<NSObject> = self.as_super().retain();
        Retained::into_super(object)
    }
}

/// Draws the menu again when what it shows has changed.
fn sync_menu(state: &mut State, controller: &Controller) {
    let status = state.engine.status();
    if state.drawn.as_ref() == Some(&status) {
        return;
    }
    let Some(bar_menu) = &state.menu else {
        return;
    };
    let (target, mtm) = (controller.target(), controller.mtm());
    bar_menu.removeAllItems();
    state.commands.clear();
    for item in menu(&status) {
        bar_menu.addItem(&draw(&item, &mut state.commands, target, mtm));
    }
    state.drawn = Some(status);
}

fn draw(
    item: &Item,
    commands: &mut Vec<Command>,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    let mut clickable = |label: &str, command: Command| {
        let key = if command == Command::Quit { "q" } else { "" };
        let drawn = menu_item(label, Some(sel!(choose:)), key, target, mtm);
        drawn.setTag(commands.len() as isize);
        commands.push(command);
        drawn
    };
    match item {
        Item::Note(label) => menu_item(label, None, "", target, mtm),
        Item::Separator => NSMenuItem::separatorItem(mtm),
        Item::Action {
            label,
            enabled,
            command,
        } => {
            let drawn = clickable(label, *command);
            drawn.setEnabled(*enabled);
            drawn
        }
        Item::Check { label, on, command } => {
            let drawn = clickable(label, *command);
            drawn.setState(*on as isize);
            drawn
        }
        Item::Submenu { label, items } => {
            let drawn = menu_item(label, None, "", target, mtm);
            drawn.setEnabled(true);
            let submenu = NSMenu::new(mtm);
            submenu.setAutoenablesItems(false);
            for item in items {
                submenu.addItem(&draw(item, commands, target, mtm));
            }
            drawn.setSubmenu(Some(&submenu));
            drawn
        }
    }
}

fn grant_access() {
    ax::trusted(true);
    let link = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(link)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

fn copy_recent_log(mtm: MainThreadMarker) {
    let path = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Logs/squiggle.log");
    let message = match std::fs::read_to_string(&path) {
        Ok(log) => {
            let recent = recent_log(&log);
            let board = objc2_app_kit::NSPasteboard::generalPasteboard();
            board.clearContents();
            board.setString_forType(&NSString::from_str(&recent), unsafe {
                objc2_app_kit::NSPasteboardTypeString
            });
            "Copied the end of Squiggle's log. Paste it into a message to report what went wrong."
                .to_string()
        }
        Err(err) => format!("Could not read {}: {err}", path.display()),
    };
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&message));
    alert.runModal();
}

fn ask_for_key(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("OpenRouter key"));
    alert.setInformativeText(&NSString::from_str(
        "Paste a key from openrouter.ai/keys. It is stored in ~/.config/squiggle/config.toml, readable only by you. Leave it empty to check on this Mac only.",
    ));
    alert.addButtonWithTitle(&NSString::from_str("Save"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    let input = NSSecureTextField::initWithFrame(
        NSSecureTextField::alloc(mtm),
        CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(300.0, 24.0)),
    );
    let current = Config::load().unwrap_or_default();
    if let Some(key) = &current.openrouter_key {
        input.setStringValue(&NSString::from_str(key));
    }
    alert.setAccessoryView(Some(&input));
    alert.window().setInitialFirstResponder(Some(&input));
    if alert.runModal() == NSAlertFirstButtonReturn {
        match parse_key(&input.stringValue().to_string()) {
            Ok(key) => update_config(|c| c.openrouter_key = key),
            Err(refusal) => {
                let warning = NSAlert::new(mtm);
                warning.setMessageText(&NSString::from_str(refusal));
                warning.runModal();
            }
        }
    }
}

fn menu_item(
    title: &str,
    action: Option<objc2::runtime::Sel>,
    key: &str,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    };
    if action.is_some() {
        unsafe { item.setTarget(Some(target)) };
    } else {
        item.setEnabled(false);
    }
    item
}

fn build_menu(state: &mut State, controller: &Controller, mtm: MainThreadMarker) {
    let bar = NSStatusBar::systemStatusBar();
    let item = bar.statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        // The system's spell-check symbol, not a logo: the name will change
        // before there is one. The Linux tray takes the icon theme's
        // equivalent (tools-check-spelling).
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("text.badge.checkmark"),
            Some(&NSString::from_str("Squiggle")),
        );
        match image {
            Some(image) => {
                image.setTemplate(true);
                button.setImage(Some(&image));
            }
            None => button.setTitle(&NSString::from_str("✓")),
        }
    }

    let bar_menu = NSMenu::new(mtm);
    bar_menu.setAutoenablesItems(false);
    item.setMenu(Some(&bar_menu));
    state.status = Some(item);
    state.menu = Some(bar_menu);
    sync_menu(state, controller);
}

/// ⌘V, ⌘C, ⌘X and ⌘A are menu key equivalents, not text-field behaviour: an
/// app with no Edit menu cannot paste into its own key dialog. Squiggle shows
/// no menu bar, but the menu still has to exist for the shortcuts to route.
pub(crate) fn edit_menu(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let main = NSMenu::new(mtm);
    let edit = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Edit"));
    for (title, action, key) in [
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
        // The dictionary window: Squiggle has no File or Window menu to carry it.
        ("Close Window", sel!(performClose:), "w"),
    ] {
        // No target: the action goes to whatever has focus, the key field.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        };
        edit.addItem(&item);
    }
    let holder = NSMenuItem::new(mtm);
    holder.setSubmenu(Some(&edit));
    main.addItem(&holder);
    main
}

pub fn run() -> anyhow::Result<()> {
    let mtm = MainThreadMarker::new().ok_or_else(|| anyhow::anyhow!("not on the main thread"))?;
    let app = NSApplication::sharedApplication(mtm);
    // No Dock icon and no menu bar of its own: Squiggle lives in the status
    // item and on top of other apps' text.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.setMainMenu(Some(&edit_menu(mtm)));

    // Asks once, the first time; macOS remembers the answer per signed binary.
    let trusted = ax::trusted(true);

    let controller = Controller::new(mtm);
    let mut state = State {
        engine: Engine::new(MacHost::new(mtm), Overlay::new(mtm, controller.handle())),
        drawn: None,
        status: None,
        menu: None,
        commands: Vec::new(),
        dictionary: None,
    };
    build_menu(&mut state, &controller, mtm);
    *controller.ivars().state.borrow_mut() = Some(state);

    let _timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            TICK.as_secs_f64(),
            controller.target(),
            sel!(tick:),
            None,
            true,
        )
    };
    // A tick the moment another app comes forward, not up to a tick later:
    // the underlines leave with the window they were drawn for.
    unsafe {
        NSWorkspace::sharedWorkspace()
            .notificationCenter()
            .addObserver_selector_name_object(
                controller.target(),
                sel!(tick:),
                Some(NSWorkspaceDidActivateApplicationNotification),
                None,
            );
    }
    note!(
        "{VERSION} ready{}",
        if trusted {
            ""
        } else {
            " (waiting for Accessibility access)"
        }
    );
    app.run();
    Ok(())
}
