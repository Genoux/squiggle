//! The menu bar app: the controller AppKit calls into, the status menu, and
//! the dialogs. The run loop itself is `surface::Engine`.

use crate::macos::ax;
use crate::macos::dictionary::Dictionary;
use crate::macos::host::MacHost;
use crate::macos::overlay::Overlay;
use crate::surface::{
    Action, DIALECTS, Engine, Status, TICK, parse_key, recent_log, update_config,
};
use gramly_core::Config;
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
use std::cell::RefCell;

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct State {
    engine: Engine<MacHost, Overlay>,
    /// What the menu was last drawn from.
    drawn: Option<Status>,
    status: Option<Retained<NSStatusItem>>,
    menu_status: Option<Retained<NSMenuItem>>,
    menu_enabled: Option<Retained<NSMenuItem>>,
    menu_ai: Option<Retained<NSMenuItem>>,
    menu_access: Option<Retained<NSMenuItem>>,
    menu_pause: Option<Retained<NSMenuItem>>,
    menu_app: Option<Retained<NSMenuItem>>,
    menu_spelling: Vec<Retained<NSMenuItem>>,
    /// Built the first time it is opened, then kept.
    dictionary: Option<Dictionary>,
}

pub struct Ivars {
    state: RefCell<Option<State>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GramlyController"]
    #[ivars = Ivars]
    pub struct Controller;

    unsafe impl NSObjectProtocol for Controller {}

    impl Controller {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &AnyObject) {
            self.with_state(|state, _| {
                state.engine.tick();
                sync_menu(state);
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

        #[unsafe(method(toggleEnabled:))]
        fn toggle_enabled(&self, _sender: &AnyObject) {
            update_config(|c| c.enabled = !c.enabled);
            self.with_state(|state, _| {
                state.engine.reset();
                sync_menu(state);
            });
        }

        #[unsafe(method(toggleAi:))]
        fn toggle_ai(&self, _sender: &AnyObject) {
            update_config(|c| c.ai = !c.ai);
            self.with_state(|state, _| {
                state.engine.clear_ai_error();
                state.engine.reset();
                sync_menu(state);
            });
        }

        #[unsafe(method(setKey:))]
        fn set_key(&self, _sender: &AnyObject) {
            ask_for_key(self.mtm());
            self.with_state(|state, _| {
                state.engine.clear_ai_error();
                state.engine.reset();
                sync_menu(state);
            });
        }

        #[unsafe(method(pauseHour:))]
        fn pause_hour(&self, _sender: &AnyObject) {
            self.with_state(|state, _| {
                state.engine.toggle_pause();
                sync_menu(state);
            });
        }

        #[unsafe(method(toggleApp:))]
        fn toggle_app(&self, _sender: &AnyObject) {
            self.with_state(|state, _| {
                state.engine.toggle_app();
                sync_menu(state);
            });
        }

        #[unsafe(method(setSpelling:))]
        fn set_spelling(&self, sender: &AnyObject) {
            let Some(&(dialect, _)) = DIALECTS.get(tag(sender)) else {
                return;
            };
            update_config(|c| c.dialect = dialect.into());
            self.with_state(|state, _| {
                // Checked again in the new spelling.
                state.engine.reset();
                sync_menu(state);
            });
        }

        #[unsafe(method(copyLog:))]
        fn copy_log(&self, _sender: &AnyObject) {
            copy_recent_log(self.mtm());
        }

        #[unsafe(method(openDictionary:))]
        fn open_dictionary(&self, _sender: &AnyObject) {
            self.with_state(|state, me| {
                let (target, mtm) = (me.target(), me.mtm());
                state
                    .dictionary
                    .get_or_insert_with(|| Dictionary::new(target, mtm))
                    .show(target, mtm);
            });
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

        #[unsafe(method(grantAccess:))]
        fn grant_access(&self, _sender: &AnyObject) {
            ax::trusted(true);
            let link = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
            if let Some(url) = NSURL::URLWithString(&NSString::from_str(link)) {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &AnyObject) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
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
fn sync_menu(state: &mut State) {
    let status = state.engine.status();
    if state.drawn.as_ref() == Some(&status) {
        return;
    }
    if let Some(item) = &state.menu_status {
        item.setTitle(&NSString::from_str(&status.line));
    }
    if let Some(item) = &state.menu_enabled {
        item.setState(status.enabled as isize);
    }
    if let Some(item) = &state.menu_ai {
        item.setState(status.ai as isize);
    }
    if let Some(item) = &state.menu_access {
        item.setHidden(status.trusted);
    }
    if let Some(item) = &state.menu_pause {
        item.setTitle(&NSString::from_str(status.pause_title()));
        item.setEnabled(status.enabled);
    }
    if let Some(item) = &state.menu_app {
        let app = status.app_item();
        item.setHidden(app.is_none());
        if let Some(app) = app {
            item.setTitle(&NSString::from_str(&app.title));
            item.setEnabled(app.enabled);
        }
    }
    for (item, (dialect, _)) in state.menu_spelling.iter().zip(DIALECTS) {
        item.setState((status.dialect == *dialect) as isize);
    }
    state.drawn = Some(status);
}

fn copy_recent_log(mtm: MainThreadMarker) {
    let path = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Logs/gramly.log");
    let message = match std::fs::read_to_string(&path) {
        Ok(log) => {
            let recent = recent_log(&log);
            let board = objc2_app_kit::NSPasteboard::generalPasteboard();
            board.clearContents();
            board.setString_forType(&NSString::from_str(&recent), unsafe {
                objc2_app_kit::NSPasteboardTypeString
            });
            "Copied the end of Gramly's log. Paste it into a message to report what went wrong."
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
        "Paste a key from openrouter.ai/keys. It is stored in ~/.config/gramly/config.toml, readable only by you. Leave it empty to check on this Mac only.",
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
    let target = controller.target();
    let bar = NSStatusBar::systemStatusBar();
    let item = bar.statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        // The system's spell-check symbol, not a logo: the name will change
        // before there is one. The Linux tray takes the icon theme's
        // equivalent (tools-check-spelling).
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("text.badge.checkmark"),
            Some(&NSString::from_str("Gramly")),
        );
        match image {
            Some(image) => {
                image.setTemplate(true);
                button.setImage(Some(&image));
            }
            None => button.setTitle(&NSString::from_str("✓")),
        }
    }

    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    let status = menu_item("Starting…", None, "", target, mtm);
    let enabled = menu_item(
        "Check as I type",
        Some(sel!(toggleEnabled:)),
        "",
        target,
        mtm,
    );
    let pause = menu_item("Pause for an hour", Some(sel!(pauseHour:)), "", target, mtm);
    let app_item = menu_item(
        "Turn off in this app",
        Some(sel!(toggleApp:)),
        "",
        target,
        mtm,
    );
    let ai = menu_item("AI suggestions", Some(sel!(toggleAi:)), "", target, mtm);
    let spelling = menu_item("Spelling", None, "", target, mtm);
    spelling.setEnabled(true);
    let choices = NSMenu::new(mtm);
    choices.setAutoenablesItems(false);
    let mut spelling_items = Vec::new();
    for (index, (_, name)) in DIALECTS.iter().enumerate() {
        let choice = menu_item(name, Some(sel!(setSpelling:)), "", target, mtm);
        choice.setTag(index as isize);
        choices.addItem(&choice);
        spelling_items.push(choice);
    }
    spelling.setSubmenu(Some(&choices));
    let log = menu_item("Copy recent log", Some(sel!(copyLog:)), "", target, mtm);
    let key = menu_item("OpenRouter key…", Some(sel!(setKey:)), "", target, mtm);
    let dictionary = menu_item(
        "Personal dictionary…",
        Some(sel!(openDictionary:)),
        "",
        target,
        mtm,
    );
    let access = menu_item(
        "Grant Accessibility access…",
        Some(sel!(grantAccess:)),
        "",
        target,
        mtm,
    );
    let about = menu_item(&format!("Gramly {VERSION}"), None, "", target, mtm);
    let quit = menu_item("Quit Gramly", Some(sel!(quit:)), "q", target, mtm);

    menu.addItem(&status);
    menu.addItem(&access);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&enabled);
    menu.addItem(&pause);
    menu.addItem(&app_item);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&ai);
    menu.addItem(&key);
    menu.addItem(&spelling);
    menu.addItem(&dictionary);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&log);
    menu.addItem(&about);
    menu.addItem(&quit);
    item.setMenu(Some(&menu));

    state.status = Some(item);
    state.menu_status = Some(status);
    state.menu_enabled = Some(enabled);
    state.menu_ai = Some(ai);
    state.menu_access = Some(access);
    state.menu_pause = Some(pause);
    state.menu_app = Some(app_item);
    state.menu_spelling = spelling_items;
    sync_menu(state);
}

/// ⌘V, ⌘C, ⌘X and ⌘A are menu key equivalents, not text-field behaviour: an
/// app with no Edit menu cannot paste into its own key dialog. Gramly shows
/// no menu bar, but the menu still has to exist for the shortcuts to route.
pub(crate) fn edit_menu(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let main = NSMenu::new(mtm);
    let edit = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Edit"));
    for (title, action, key) in [
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
        // The dictionary window: Gramly has no File or Window menu to carry it.
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
    // No Dock icon and no menu bar of its own: Gramly lives in the status
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
        menu_status: None,
        menu_enabled: None,
        menu_ai: None,
        menu_access: None,
        menu_pause: None,
        menu_app: None,
        menu_spelling: Vec::new(),
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
