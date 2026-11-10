use super::edit::{self, ANCESTORS};
use super::hypr::Window;
use crate::surface::{self, Rect, Write, skip};
use atspi_proxies::accessible::AccessibleProxyBlocking;
use atspi_proxies::application::ApplicationProxyBlocking;
use atspi_proxies::bus::{BusProxyBlocking, StatusProxyBlocking};
use atspi_proxies::collection::CollectionProxyBlocking;
use atspi_proxies::common::{
    CoordType, Interface, InterfaceSet, MatchType, ObjectMatchRule, ObjectRefOwned, Role,
    SortOrder, State,
};
use atspi_proxies::component::ComponentProxyBlocking;
use atspi_proxies::document::DocumentProxyBlocking;
use atspi_proxies::hyperlink::HyperlinkProxyBlocking;
use atspi_proxies::hypertext::HypertextProxyBlocking;
use atspi_proxies::registry::RegistryProxyBlocking;
use atspi_proxies::text::TextProxyBlocking;
use padding::{app_offset, strip_padding};
use squiggle_core::off::Place;
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use zbus::MatchRule;
use zbus::blocking::connection::Builder;
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator};
use zbus::message::{Message, Type as MessageType};
use zbus::names::BusName;
use zbus::zvariant::OwnedValue;

/// Every call is IPC into another app; zbus waits 25s by default.
const CALL_TIMEOUT: Duration = Duration::from_millis(150);
/// Starting the accessibility bus can launch its daemon first.
const SETUP_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a field is left alone after one of its calls took its whole
/// deadline: the app is busy or hung, and every further call would freeze
/// the main loop for another [`CALL_TIMEOUT`].
const COOLDOWN: Duration = Duration::from_secs(1);
const WALK_LIMIT: usize = 5000;
/// A web document holding a focused field is focused and editable too.
const FOCUS_MATCHES: i32 = 5;
const FOCUS_EVENT: &str = "object:state-changed:focused";
const REGISTRY_BUS: &str = "org.a11y.atspi.Registry";
const ROOT_PATH: &str = "/org/a11y/atspi/accessible/root";

/// Why Squiggle stays out of `window`: its own, or an app it never reads.
pub fn skipped(window: &Window) -> Option<String> {
    if window.pid == std::process::id() {
        return Some("focus is in Squiggle itself".into());
    }
    skip::app(&window.class).map(|kind| format!("{} is {kind}, skipped", window.class))
}

/// One character's box as an app reported it, and the frame it asked in.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CharBox {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    space: CoordType,
}

/// The span between two character boxes on the screen. Whatever frame the
/// app answered in, its origin is the window's top-left, because Wayland
/// hides the window's position from the client: GTK4 refuses `Screen`,
/// Chromium and Gecko accept it and still answer from the window.
fn place(first: CharBox, last: CharBox, at: (f64, f64)) -> Option<Rect> {
    let same_line = (first.y - last.y).abs() <= first.h.min(last.h) / 2;
    if first.space != last.space || first.w <= 0 || last.w <= 0 || !same_line {
        return None;
    }
    let left = first.x.min(last.x);
    let top = first.y.min(last.y);
    let right = (first.x + first.w).max(last.x + last.w);
    let bottom = (first.y + first.h).max(last.y + last.h);
    Some(Rect {
        x: at.0 + f64::from(left),
        y: at.1 + f64::from(top),
        w: f64::from(right - left),
        h: f64::from(bottom - top),
    })
}

/// Whether an `object:state-changed:focused` signal says the source gained
/// (`Some(true)`) or lost (`Some(false)`) focus.
fn focus_change(kind: &str, detail1: i32) -> Option<bool> {
    match (kind, detail1) {
        ("focused", 1) => Some(true),
        ("focused", 0) => Some(false),
        _ => None,
    }
}

fn cooling(slow_until: Option<Instant>, now: Instant) -> bool {
    slow_until.is_some_and(|until| now < until)
}

/// The frames to ask for a box, in order: the one that answered last, or
/// Screen then Window. GTK4 refuses Screen, and asked first for every
/// character it refused it every time.
fn spaces(answered: Option<CoordType>) -> Vec<CoordType> {
    answered.map_or_else(
        || vec![CoordType::Screen, CoordType::Window],
        |space| vec![space],
    )
}

/// Where a field came from, for `doctor`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Via {
    Event,
    Walk,
}

impl Via {
    pub fn name(self) -> &'static str {
        match self {
            Via::Event => "focus event",
            Via::Walk => "tree walk",
        }
    }
}

/// A focus event's source: the app's bus name and the object's path.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    bus: String,
    path: String,
}

/// The latest object that announced it gained focus, kept by a thread
/// listening on the accessibility bus. A tree walk per tick is too slow:
/// a Zen window with 3086 nodes took 0.44s.
pub struct Focus(Arc<Mutex<Option<Source>>>);

impl Focus {
    pub fn latest(&self) -> Option<Source> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub fn enable() -> zbus::Result<bool> {
    let status =
        StatusProxyBlocking::new(&Builder::session()?.method_timeout(SETUP_TIMEOUT).build()?)?;
    let was_on = status.is_enabled()?;
    if !was_on {
        status.set_is_enabled(true)?;
    }
    Ok(was_on)
}

pub fn connect() -> zbus::Result<Connection> {
    let session = Builder::session()?.method_timeout(SETUP_TIMEOUT).build()?;
    let address = BusProxyBlocking::new(&session)?.get_address()?;
    Builder::address(address.as_str())?
        .method_timeout(CALL_TIMEOUT)
        .build()
}

pub fn listen(conn: &Connection) -> zbus::Result<Focus> {
    RegistryProxyBlocking::new(conn)?.register_event(FOCUS_EVENT)?;
    let rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .interface("org.a11y.atspi.Event.Object")?
        .member("StateChanged")?
        .build();
    let events = MessageIterator::for_match_rule(rule, conn, None)?;
    let latest = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&latest);
    std::thread::spawn(move || {
        for message in events.flatten() {
            let Some((source, gained)) = focus_signal(&message) else {
                continue;
            };
            let mut current = sink.lock().unwrap_or_else(PoisonError::into_inner);
            if gained {
                *current = Some(source);
            } else if current.as_ref() == Some(&source) {
                *current = None;
            }
        }
    });
    Ok(Focus(latest))
}

fn focus_signal(message: &Message) -> Option<(Source, bool)> {
    type Body = (String, i32, i32, OwnedValue, HashMap<String, OwnedValue>);
    let (kind, detail1, ..) = message.body().deserialize::<Body>().ok()?;
    let gained = focus_change(&kind, detail1)?;
    let header = message.header();
    let source = Source {
        bus: header.sender()?.to_string(),
        path: header.path()?.to_string(),
    };
    Some((source, gained))
}

/// The focused text field of the app owning `window`, or why there is none.
pub fn explain_focus(
    conn: &Connection,
    focus: &Focus,
    window: &Window,
) -> Result<(Field, Via), String> {
    if let Some(why) = skipped(window) {
        return Err(why);
    }
    let at = Rc::new(Cell::new(window.at));
    let from_event = focus
        .latest()
        .and_then(|source| from_event(conn, &source, window.pid, &at))
        .flatten();
    let (field, via) = match from_event {
        Some(field) => (field, Via::Event),
        None => (walk(conn, window, &at, &AtomicBool::new(false))?, Via::Walk),
    };
    if let Some(kind) = field.skipped() {
        return Err(format!("{kind}, skipped"));
    }
    Ok((field, via))
}

/// The pid check is what ties a field to the front window: an event can come
/// from any app on the bus, and the Chromium browser process owns both.
/// `Some(None)` is a focused control that takes no text; `None` cannot tell
/// (no answer, another app's event, or a page whose field a walk must find).
pub fn from_event(
    conn: &Connection,
    source: &Source,
    pid: u32,
    at: &Rc<Cell<(f64, f64)>>,
) -> Option<Option<Field>> {
    if pid_of(conn, &source.bus)? != pid {
        return None;
    }
    let field = Field::from_source(conn, source, at);
    if field.qualifies()? {
        return Some(Some(field));
    }
    if let Some(inside) = Field::new(conn, &source.bus, ROOT_PATH, at).focused_in_app() {
        return Some(Some(inside));
    }
    field.is_focused_control().then_some(None)
}

/// Looks through the tree of the app owning `window` for the focused text
/// field, giving up when `cancel` is set: the host dropped this walk.
pub fn walk(
    conn: &Connection,
    window: &Window,
    at: &Rc<Cell<(f64, f64)>>,
    cancel: &AtomicBool,
) -> Result<Field, String> {
    let app = app_of(conn, window, at)?;
    let root = Field::new(conn, app.name_as_str().unwrap_or_default(), ROOT_PATH, at);
    if let Some(field) = root.focused_in_app() {
        return Ok(field);
    }
    let mut queue = VecDeque::from([(app.clone(), 0)]);
    let (mut visited, mut resolved_below_frames) = (0, 0);
    while let Some((node, depth)) = queue.pop_front() {
        let Some(bus) = node.name_as_str().filter(|_| !node.is_null()) else {
            continue;
        };
        if cancel.load(Ordering::Relaxed) {
            return Err("walk cancelled".into());
        }
        if visited == WALK_LIMIT {
            return Err(format!(
                "gave up after {WALK_LIMIT} nodes in {}",
                window.class
            ));
        }
        visited += 1;
        let field = Field::new(conn, bus, node.path_as_str(), at);
        match field.qualifies() {
            Some(true) => return Ok(field),
            Some(false) => {
                resolved_below_frames += usize::from(depth >= 2);
                queue.extend(field.children().into_iter().map(|child| (child, depth + 1)));
            }
            None => {}
        }
    }
    let toolkit = root.toolkit();
    Err(no_field(
        window,
        toolkit.as_deref(),
        visited,
        resolved_below_frames,
    ))
}

fn app_of(
    conn: &Connection,
    window: &Window,
    at: &Rc<Cell<(f64, f64)>>,
) -> Result<ObjectRefOwned, String> {
    Field::new(conn, REGISTRY_BUS, ROOT_PATH, at)
        .children()
        .into_iter()
        .find(|app| app.name_as_str().and_then(|bus| pid_of(conn, bus)) == Some(window.pid))
        .ok_or_else(|| {
            format!(
                "{} (pid {}) is not on the accessibility bus; restart it now that accessibility is on",
                window.class, window.pid
            )
        })
}

fn no_field(window: &Window, toolkit: Option<&str>, visited: usize, resolved: usize) -> String {
    match (toolkit, resolved) {
        (Some("Chromium"), 0) => format!(
            "{} is Chromium with no accessibility tree; start it with --force-renderer-accessibility",
            window.class
        ),
        _ => format!(
            "no focused text field in {} ({visited} nodes looked at)",
            window.class
        ),
    }
}

fn pid_of(conn: &Connection, bus: &str) -> Option<u32> {
    DBusProxy::new(conn)
        .ok()?
        .get_connection_unix_process_id(BusName::try_from(bus).ok()?)
        .ok()
}

/// An accessible object on the accessibility bus.
pub struct Field {
    conn: Connection,
    bus: String,
    path: String,
    /// The window's top-left, which Hyprland moves without the text changing:
    /// one cell the host refreshes each tick, shared with every field.
    at: Rc<Cell<(f64, f64)>>,
    /// What `value` last read, with the padding it removed; see
    /// [`strip_padding`].
    text: RefCell<String>,
    padding: RefCell<Vec<usize>>,
    /// The blocks the value was read from, one line each, when the field's
    /// own text is only embedded objects; see [`blocks`]. Empty otherwise.
    blocks: RefCell<Vec<Rc<Field>>>,
    /// The block `select` chose, whose caret is put back.
    chosen: RefCell<Option<Rc<Field>>>,
    /// Whether this is a web field, decided once; see [`Field::is_web`].
    web: OnceCell<bool>,
    /// Set once a write was ignored: fixes are pasted from then on. A field
    /// taken as web only because it did not answer is asked again next time.
    paste_only: Cell<bool>,
    /// Until when calls are not made; see [`cooling`].
    slow_until: Cell<Option<Instant>>,
    /// The frame the app last answered a box in; see [`spaces`].
    space: Cell<Option<CoordType>>,
    /// The caret and the selection `select_range` made, kept so a refused
    /// fix can put the caret back and the paste can be checked once more.
    caret: Cell<Option<i32>>,
    selected: Cell<Option<(i32, i32)>>,
}

/// Uncached: zbus would otherwise fetch every property and subscribe to
/// their changes for each proxy built, and apps on the accessibility bus do
/// not announce property changes, so a cached value goes stale.
macro_rules! proxy {
    ($field:expr, $kind:ident) => {
        proxy!($field, $kind, $field.path.as_str())
    };
    ($field:expr, $kind:ident, $path:expr) => {
        $kind::builder(&$field.conn)
            .destination($field.bus.as_str())
            .ok()?
            .path($path)
            .ok()?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .ok()?
    };
}

mod blocks;
mod padding;
mod write;

impl Field {
    fn new(conn: &Connection, bus: &str, path: &str, at: &Rc<Cell<(f64, f64)>>) -> Self {
        Self {
            conn: conn.clone(),
            bus: bus.to_owned(),
            path: path.to_owned(),
            at: Rc::clone(at),
            text: RefCell::new(String::new()),
            padding: RefCell::new(Vec::new()),
            blocks: RefCell::new(Vec::new()),
            chosen: RefCell::new(None),
            web: OnceCell::new(),
            paste_only: Cell::new(false),
            slow_until: Cell::new(None),
            space: Cell::new(None),
            caret: Cell::new(None),
            selected: Cell::new(None),
        }
    }

    pub fn from_source(conn: &Connection, source: &Source, at: &Rc<Cell<(f64, f64)>>) -> Self {
        Self::new(conn, &source.bus, &source.path, at)
    }

    pub fn source(&self) -> Source {
        Source {
            bus: self.bus.clone(),
            path: self.path.clone(),
        }
    }

    /// What the field is, when it is never read: a password field, or one
    /// whose page says it holds no prose.
    pub fn skipped(&self) -> Option<&'static str> {
        if self.role() == Some(Role::PasswordText) {
            return Some("a password field");
        }
        let attributes = proxy!(self, AccessibleProxyBlocking)
            .get_attributes()
            .unwrap_or_default();
        let named = |key: &str| attributes.get(key).map_or("", String::as_str);
        skip::field(named("xml-roles"), named("text-input-type"), named("class"))
    }

    /// Makes `call` unless this field is cooling down, and starts the
    /// cooldown when it took its whole deadline.
    fn timed<T>(&self, call: impl FnOnce() -> Option<T>) -> Option<T> {
        if cooling(self.slow_until.get(), Instant::now()) {
            return None;
        }
        let asked = Instant::now();
        let answer = call();
        if asked.elapsed() >= CALL_TIMEOUT {
            self.slow_until.set(Some(Instant::now() + COOLDOWN));
        }
        answer
    }

    /// `None` when the object does not answer. Web documents are excluded:
    /// they are focused and editable whenever a page is, and the field inside
    /// is the one wanted.
    fn qualifies(&self) -> Option<bool> {
        self.timed(|| {
            let object = proxy!(self, AccessibleProxyBlocking);
            let state = object.get_state().ok()?;
            Some(
                state.contains(State::Focused | State::Editable)
                    && object.get_interfaces().ok()?.contains(Interface::Text)
                    && object.get_role().ok()? != Role::DocumentWeb,
            )
        })
    }

    /// The focused text field anywhere in this object's app, in one call,
    /// where the toolkit offers `Collection` (Gecko does). Zen reports a
    /// childless panel of its window as focused whenever the window comes
    /// forward and says nothing of the field still focused inside the page;
    /// taken as a control, the field was never looked for. Unset criteria
    /// must be `NA`: at-spi2-atk matches nothing on `Invalid`.
    fn focused_in_app(&self) -> Option<Field> {
        let rule = ObjectMatchRule::builder()
            .states([State::Focused, State::Editable], MatchType::All)
            .attributes(HashMap::new(), MatchType::NA)
            .roles(&[], MatchType::NA)
            .interfaces(InterfaceSet::empty(), MatchType::NA)
            .build();
        let matches = self.timed(|| {
            proxy!(self, CollectionProxyBlocking, ROOT_PATH)
                .get_matches(rule, SortOrder::Canonical, FOCUS_MATCHES, false)
                .ok()
        })?;
        matches.into_iter().find_map(|node| {
            let field = Field::new(
                &self.conn,
                node.name_as_str()?,
                node.path_as_str(),
                &self.at,
            );
            field.qualifies()?.then_some(field)
        })
    }

    fn is_focused_control(&self) -> bool {
        self.timed(|| {
            let object = proxy!(self, AccessibleProxyBlocking);
            Some(
                object.get_state().ok()?.contains(State::Focused)
                    && object.get_role().ok()? != Role::DocumentWeb,
            )
        })
        .unwrap_or(false)
    }

    fn role(&self) -> Option<Role> {
        proxy!(self, AccessibleProxyBlocking).get_role().ok()
    }

    fn parent(&self) -> Option<ObjectRefOwned> {
        proxy!(self, AccessibleProxyBlocking).parent().ok()
    }

    /// See [`edit::is_web`]. A field that cannot be asked about is taken as
    /// web: that way fixes are pasted over a confirmed selection, never
    /// written.
    pub fn is_web(&self) -> bool {
        if let Some(web) = self.web.get() {
            return *web;
        }
        let Some(web) = self.timed(|| self.read_web()) else {
            return true;
        };
        *self.web.get_or_init(|| web)
    }

    fn read_web(&self) -> Option<bool> {
        let toolkit = self.toolkit()?;
        let editable = proxy!(self, AccessibleProxyBlocking)
            .get_interfaces()
            .ok()?
            .contains(Interface::EditableText);
        let ancestors = if edit::web_toolkit(&toolkit) {
            Vec::new()
        } else {
            self.ancestor_roles()?
        };
        Some(edit::is_web(&toolkit, editable, &ancestors))
    }

    /// The roles above this field, up to a web document.
    fn ancestor_roles(&self) -> Option<Vec<Role>> {
        let mut ancestors = Vec::new();
        let mut node = self.parent()?;
        while ancestors.len() < ANCESTORS && !node.is_null() {
            let above = Field::new(
                &self.conn,
                node.name_as_str()?,
                node.path_as_str(),
                &self.at,
            );
            let role = above.role()?;
            ancestors.push(role);
            if role == Role::DocumentWeb {
                break;
            }
            node = above.parent()?;
        }
        Some(ancestors)
    }

    fn children(&self) -> Vec<ObjectRefOwned> {
        (|| proxy!(self, AccessibleProxyBlocking).get_children().ok())().unwrap_or_default()
    }

    /// The app's name, from the bus's root object: "Firefox", "Slack".
    fn app_name(&self) -> Option<String> {
        proxy!(self, AccessibleProxyBlocking, ROOT_PATH)
            .name()
            .ok()
            .filter(|name| !name.trim().is_empty())
    }

    /// The host of the page a web field is on, from the web document above
    /// it.
    fn page_host(&self) -> Option<String> {
        let mut node = self.parent()?;
        for _ in 0..ANCESTORS {
            if node.is_null() {
                return None;
            }
            let above = Field::new(
                &self.conn,
                node.name_as_str()?,
                node.path_as_str(),
                &self.at,
            );
            if above.role()? == Role::DocumentWeb {
                let url = proxy!(above, DocumentProxyBlocking)
                    .get_attribute_value("DocURL")
                    .ok()?;
                return host_of(&url);
            }
            node = above.parent()?;
        }
        None
    }

    /// Its id and its first label: the DOM id Gecko and Chromium list among
    /// its attributes, or the toolkit's own; its name, description or
    /// placeholder.
    fn names(&self) -> Option<(String, String)> {
        let object = proxy!(self, AccessibleProxyBlocking);
        let attributes = object.get_attributes().unwrap_or_default();
        let id = attributes
            .get("id")
            .cloned()
            .or_else(|| object.accessible_id().ok())
            .unwrap_or_default();
        let label = [
            object.name().ok(),
            object.description().ok(),
            attributes.get("placeholder-text").cloned(),
        ]
        .into_iter()
        .flatten()
        .map(|label| label.trim().to_string())
        .find(|label| !label.is_empty())
        .unwrap_or_default();
        Some((id.trim().to_string(), label))
    }

    /// The toolkit of the app this field is in, from the bus's root object.
    fn toolkit(&self) -> Option<String> {
        proxy!(self, ApplicationProxyBlocking, ROOT_PATH)
            .toolkit_name()
            .ok()
    }

    fn raw_text(&self) -> Option<String> {
        let text = proxy!(self, TextProxyBlocking);
        text.get_text(0, text.character_count().ok()?).ok()
    }

    /// The text without its padding, kept with the padding it removed; or,
    /// when the text is only embedded objects, its blocks' texts, one line
    /// each. A block that does not answer is a miss, not a blank field:
    /// read as its bare U+FFFC, Slack's box flipped between its 34
    /// characters and one while its composer was rebuilt, and each flip
    /// cleared the underlines.
    fn read_value(&self) -> Option<String> {
        let raw = self.raw_text()?;
        let blocks = if blocks::only_objects(&raw) {
            self.text_objects()?
        } else {
            Vec::new()
        };
        let (text, padding) = if blocks.is_empty() {
            strip_padding(&raw)
        } else {
            let lines = blocks
                .iter()
                .map(|block| block.read_value())
                .collect::<Option<Vec<_>>>()?;
            (lines.join("\n"), Vec::new())
        };
        *self.padding.borrow_mut() = padding;
        *self.blocks.borrow_mut() = blocks;
        *self.text.borrow_mut() = text.clone();
        Some(text)
    }

    /// The embedded objects that hold text, in order; `None` when the app
    /// did not answer.
    // ponytail: every block is read on every poll, a few calls each; a long
    // document may reach CALL_TIMEOUT. Read only the blocks that changed if so.
    fn text_objects(&self) -> Option<Vec<Rc<Field>>> {
        let hypertext = proxy!(self, HypertextProxyBlocking);
        let objects = (0..hypertext.get_n_links().ok()?)
            .map(|index| {
                let link = hypertext.get_link(index).ok()?;
                let object = proxy!(self, HyperlinkProxyBlocking, link.path_as_str())
                    .get_object(0)
                    .ok()?;
                let block = Field::new(
                    &self.conn,
                    object.name_as_str()?,
                    object.path_as_str(),
                    &self.at,
                );
                let holds_text = proxy!(block, AccessibleProxyBlocking)
                    .get_interfaces()
                    .ok()?
                    .contains(Interface::Text);
                Some(holds_text.then(|| Rc::new(block)))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(objects.into_iter().flatten().collect())
    }

    fn has_blocks(&self) -> bool {
        !self.blocks.borrow().is_empty()
    }

    /// The block holding `len` characters from `start` of the value last
    /// read, and where they start in it.
    fn block_at(&self, start: usize, len: usize) -> Option<(Rc<Field>, usize)> {
        let blocks = self.blocks.borrow();
        let lines: Vec<usize> = blocks
            .iter()
            .map(|block| block.text.borrow().chars().count())
            .collect();
        let (index, local) = blocks::locate(&lines, start, len)?;
        Some((Rc::clone(&blocks[index]), local))
    }

    /// A character's box, in the frame that answered last. A frame that
    /// stops answering is forgotten, and both are tried again.
    fn extents(
        &self,
        ask: impl Fn(CoordType) -> zbus::Result<(i32, i32, i32, i32)>,
    ) -> Option<CharBox> {
        let found = spaces(self.space.get()).into_iter().find_map(|space| {
            let (x, y, w, h) = ask(space).ok()?;
            Some(CharBox { x, y, w, h, space })
        });
        self.space.set(found.map(|found| found.space));
        found
    }
}

impl surface::Field for Field {
    type Handle = (String, String);

    fn handle(&self) -> (String, String) {
        (self.bus.clone(), self.path.clone())
    }

    /// The app, the page's host for a web field, its id and its first
    /// label.
    fn place(&self) -> Place {
        let app = self.timed(|| self.app_name()).unwrap_or_default();
        let site = if self.is_web() {
            self.timed(|| self.page_host()).unwrap_or_default()
        } else {
            String::new()
        };
        let (id, label) = self.timed(|| self.names()).unwrap_or_default();
        let who = if site.is_empty() {
            app.clone()
        } else {
            site.clone()
        };
        Place::new(app, site, id, label, &who)
    }

    fn value(&self) -> Option<String> {
        self.timed(|| self.read_value())
    }

    fn visible_frame(&self) -> Option<Rect> {
        let frame = self.timed(|| {
            let component = proxy!(self, ComponentProxyBlocking);
            self.extents(|space| component.get_extents(space))
        })?;
        place(frame, frame, self.at.get())
    }

    /// `start` and `len` count characters, as `Fix` offsets do. GetRangeExtents
    /// is not used: GTK4 answers with the first character's box only and
    /// Gecko with partial or empty ones.
    fn bounds(&self, start: usize, len: usize) -> Option<Rect> {
        if self.has_blocks() {
            let (block, local) = self.block_at(start, len)?;
            return surface::Field::bounds(&*block, local, len);
        }
        let text = proxy!(self, TextProxyBlocking);
        let padding = self.padding.borrow();
        let character = |offset: usize| {
            let offset = i32::try_from(app_offset(&padding, offset)).ok()?;
            self.timed(|| self.extents(|space| text.get_character_extents(offset, space)))
        };
        let last = start.checked_add(len.checked_sub(1)?)?;
        place(character(start)?, character(last)?, self.at.get())
    }

    /// Selects the range of the value last read and waits up to
    /// [`SETTLE`](crate::surface::write::SETTLE) for the app to report exactly
    /// that range, holding exactly that text: Gecko applies a selection a
    /// moment late.
    fn select(&self, start: usize, len: usize) -> bool {
        if !self.has_blocks() {
            self.chosen.take();
            return self.select_range(start, len).unwrap_or(false);
        }
        let Some((block, local)) = self.block_at(start, len) else {
            return false;
        };
        let selected = surface::Field::select(&*block, local, len);
        *self.chosen.borrow_mut() = Some(block);
        selected
    }

    fn still_selected(&self) -> bool {
        self.selection_holds()
    }

    fn deselect(&self) {
        self.put_caret_back();
    }

    /// Writes through `EditableText` where the app honours it. Web fields
    /// are left alone, as on macOS: a direct write in Zen replaced a whole
    /// sentence with one word. So is a field read through its blocks: its
    /// own offsets count each block as one character. `Ignored` hands them
    /// to the paste.
    fn replace(&self, start: usize, len: usize, text: &str, before: &str, expected: &str) -> Write {
        if self.paste_only.get() || self.is_web() || self.has_blocks() {
            return Write::Ignored;
        }
        let outcome = self.write_native(start, len, text, before, expected);
        if outcome == Write::Ignored {
            self.paste_only.set(true);
        }
        outcome
    }
}

impl PartialEq for Field {
    fn eq(&self, other: &Self) -> bool {
        self.bus == other.bus && self.path == other.path
    }
}

/// The host part of a URL: "github.com" from "https://github.com/a/b".
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then(|| host.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_known_by_its_host() {
        assert_eq!(
            host_of("https://github.com/Genoux/squiggle/pull/7#top").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            host_of("http://user@Example.org:8080/?q=1").as_deref(),
            Some("example.org")
        );
        assert_eq!(host_of("about:blank"), None);
        assert_eq!(host_of("file:///home/john/a.html"), None);
    }

    fn window() -> Window {
        Window {
            address: "0x1".into(),
            pid: 1,
            class: "zen".into(),
            at: (100.0, 50.0),
            size: (800.0, 600.0),
        }
    }

    fn char_box(x: i32, y: i32, w: i32, space: CoordType) -> CharBox {
        CharBox {
            x,
            y,
            w,
            h: 20,
            space,
        }
    }

    #[test]
    fn spans_the_first_and_last_character_from_the_window_origin() {
        let first = char_box(10, 30, 8, CoordType::Window);
        let last = char_box(60, 30, 9, CoordType::Window);
        assert_eq!(
            place(first, last, window().at),
            Some(Rect {
                x: 110.0,
                y: 80.0,
                w: 59.0,
                h: 20.0
            })
        );
    }

    #[test]
    fn a_screen_answer_is_also_from_the_window_origin() {
        let only = char_box(10, 30, 8, CoordType::Screen);
        assert_eq!(
            place(only, only, window().at),
            Some(Rect {
                x: 110.0,
                y: 80.0,
                w: 8.0,
                h: 20.0
            })
        );
    }

    #[test]
    fn a_zero_width_box_is_no_answer() {
        let good = char_box(10, 30, 8, CoordType::Window);
        let empty = char_box(60, 30, 0, CoordType::Window);
        assert_eq!(place(good, empty, window().at), None);
        assert_eq!(place(empty, good, window().at), None);
    }

    #[test]
    fn boxes_in_different_frames_do_not_combine() {
        let first = char_box(10, 30, 8, CoordType::Window);
        let last = char_box(60, 30, 8, CoordType::Screen);
        assert_eq!(place(first, last, window().at), None);
    }

    #[test]
    fn boxes_on_different_lines_do_not_combine() {
        let first = char_box(10, 30, 8, CoordType::Window);
        let last = char_box(60, 52, 8, CoordType::Window);
        assert_eq!(place(first, last, window().at), None);
    }

    #[test]
    fn a_taller_glyph_on_the_same_line_still_combines() {
        let first = char_box(10, 30, 8, CoordType::Window);
        let emoji = CharBox {
            y: 28,
            h: 24,
            ..char_box(60, 0, 8, CoordType::Window)
        };
        assert_eq!(
            place(first, emoji, window().at),
            Some(Rect {
                x: 110.0,
                y: 78.0,
                w: 58.0,
                h: 24.0
            })
        );
    }

    #[test]
    fn only_the_focused_kind_counts() {
        assert_eq!(focus_change("focused", 1), Some(true));
        assert_eq!(focus_change("focused", 0), Some(false));
        assert_eq!(focus_change("selected", 1), None);
        assert_eq!(focus_change("focused", 2), None);
    }

    #[test]
    fn a_hung_app_is_left_alone_until_the_cooldown_ends() {
        let now = Instant::now();
        let until = now + COOLDOWN;
        assert!(cooling(Some(until), now));
        assert!(cooling(Some(until), until - Duration::from_millis(1)));
        assert!(!cooling(Some(until), until));
        assert!(!cooling(None, now));
    }

    #[test]
    fn a_frame_that_answered_is_asked_alone() {
        assert_eq!(spaces(None), [CoordType::Screen, CoordType::Window]);
        assert_eq!(spaces(Some(CoordType::Window)), [CoordType::Window]);
        assert_eq!(spaces(Some(CoordType::Screen)), [CoordType::Screen]);
    }
}
