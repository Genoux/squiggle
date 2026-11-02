use super::*;
use crate::surface::write::{Paste, Refusal};
use crate::surface::{Bit, Card, Point, Write};
use gramly_core::Source;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const TEXT: &str = "i went to paris";
const FIELD: Rect = Rect {
    x: 0.0,
    y: 90.0,
    w: 400.0,
    h: 40.0,
};

#[derive(Clone)]
struct FakeField {
    text: Rc<RefCell<String>>,
    /// Its label: empty, and it cannot be known again after a restart.
    label: &'static str,
}

impl PartialEq for FakeField {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.text, &other.text)
    }
}

impl Field for FakeField {
    type Handle = usize;

    fn handle(&self) -> usize {
        Rc::as_ptr(&self.text) as usize
    }

    fn place(&self) -> Place {
        Place::new(
            "app.id".into(),
            String::new(),
            String::new(),
            self.label.into(),
            "App",
        )
    }

    fn value(&self) -> Option<String> {
        Some(self.text.borrow().clone())
    }

    fn visible_frame(&self) -> Option<Rect> {
        Some(FIELD)
    }

    fn bounds(&self, start: usize, len: usize) -> Option<Rect> {
        Some(Rect {
            x: 10.0 + start as f64 * 8.0,
            y: 100.0,
            w: len as f64 * 8.0,
            h: 16.0,
        })
    }

    fn select(&self, _start: usize, _len: usize) -> bool {
        true
    }

    fn replace(&self, start: usize, len: usize, text: &str, _: &str, _: &str) -> Write {
        let chars: Vec<char> = self.text.borrow().chars().collect();
        let rewritten: String = chars[..start]
            .iter()
            .copied()
            .chain(text.chars())
            .chain(chars[start + len..].iter().copied())
            .collect();
        *self.text.borrow_mut() = rewritten;
        Write::Done
    }
}

struct NoPaste;

impl Paste for NoPaste {
    fn stage(&self, _text: &str) -> Result<(), Refusal> {
        Ok(())
    }

    fn send(&self) -> Result<(), Refusal> {
        Ok(())
    }

    fn unstage(&self) {}

    fn delete_selection(&self) -> Result<(), Refusal> {
        Ok(())
    }
}

struct FakeHost {
    field: FakeField,
    config: Config,
    trusted: bool,
    pointer: Rc<Cell<Point>>,
    focused: Rc<Cell<bool>>,
    elsewhere: Rc<Cell<bool>>,
    undos: Rc<Cell<u32>>,
    clocks: Rc<Cell<u32>>,
    /// `off.txt`, in memory.
    saved_off: Rc<RefCell<Vec<Place>>>,
}

impl Host for FakeHost {
    type Field = FakeField;

    fn config(&self) -> Config {
        self.config.clone()
    }

    fn trusted(&self) -> bool {
        self.trusted
    }

    fn front(&self) -> Option<(String, String)> {
        Some(("app.id".into(), "App".into()))
    }

    fn load_off(&self) -> Vec<Place> {
        self.saved_off.borrow().clone()
    }

    fn save_off(&self, places: &[Place]) -> anyhow::Result<()> {
        *self.saved_off.borrow_mut() = places.to_vec();
        Ok(())
    }

    fn focus(&mut self, current: Option<&FakeField>) -> Focus<FakeField> {
        match (self.focused.get(), current) {
            _ if self.elsewhere.get() => Focus::Elsewhere,
            (false, _) => Focus::Unknown,
            (true, Some(held)) if *held == self.field => Focus::Same,
            (true, _) => Focus::Field(self.field.clone()),
        }
    }

    fn pointer(&self) -> Point {
        self.pointer.get()
    }

    fn clipboard(&self) -> &dyn Paste {
        &NoPaste
    }

    fn undo(&self) {
        self.undos.set(self.undos.get() + 1);
    }

    fn clock(&self, _at: Instant) -> String {
        self.clocks.set(self.clocks.get() + 1);
        "3:00 PM".into()
    }
}

#[derive(Default)]
struct Drawn {
    marks: Vec<(Rect, Vec<Mark>)>,
    card: Option<(usize, Card, Rect)>,
    hidden_marks: u32,
    cards_shown: u32,
    menu_toggles: u32,
}

struct FakeOverlay(Rc<RefCell<Drawn>>);

impl Overlay for FakeOverlay {
    fn marks(&mut self, area: Rect, marks: Vec<Mark>) {
        self.0.borrow_mut().marks.push((area, marks));
    }

    fn hide_marks(&mut self) {
        self.0.borrow_mut().hidden_marks += 1;
    }

    fn show_card(&mut self, index: usize, card: &Card, word: Rect) {
        let mut drawn = self.0.borrow_mut();
        drawn.card = Some((index, card.clone(), word));
        drawn.cards_shown += 1;
    }

    fn hide_card(&mut self) {
        self.0.borrow_mut().card = None;
    }

    fn card_frame(&self) -> Option<Rect> {
        self.0.borrow().card.as_ref().map(|(_, _, word)| Rect {
            x: word.x,
            y: word.y + word.h + 4.0,
            w: 200.0,
            h: 100.0,
        })
    }

    fn card_rows(&self) -> Vec<(Action, Rect)> {
        Vec::new()
    }

    fn hover(&mut self, _pointer: Point) {}

    fn toggle_menu(&mut self) {
        self.0.borrow_mut().menu_toggles += 1;
    }

    fn menu_frame(&self) -> Option<Rect> {
        None
    }
}

struct Rig {
    engine: Engine<FakeHost, FakeOverlay>,
    drawn: Rc<RefCell<Drawn>>,
    pointer: Rc<Cell<Point>>,
    text: Rc<RefCell<String>>,
    focused: Rc<Cell<bool>>,
    elsewhere: Rc<Cell<bool>>,
    undos: Rc<Cell<u32>>,
    clocks: Rc<Cell<u32>>,
    saved_off: Rc<RefCell<Vec<Place>>>,
}

fn rig(config: Config, trusted: bool) -> Rig {
    rig_labelled(config, trusted, "Message")
}

fn rig_labelled(config: Config, trusted: bool, label: &'static str) -> Rig {
    let text = Rc::new(RefCell::new(TEXT.to_string()));
    let pointer = Rc::new(Cell::new(Point { x: 900.0, y: 900.0 }));
    let drawn = Rc::new(RefCell::new(Drawn::default()));
    let focused = Rc::new(Cell::new(true));
    let elsewhere = Rc::new(Cell::new(false));
    let undos = Rc::new(Cell::new(0));
    let clocks = Rc::new(Cell::new(0));
    let saved_off = Rc::new(RefCell::new(Vec::new()));
    let host = FakeHost {
        field: FakeField {
            text: text.clone(),
            label,
        },
        config,
        trusted,
        pointer: pointer.clone(),
        focused: focused.clone(),
        elsewhere: elsewhere.clone(),
        undos: undos.clone(),
        clocks: clocks.clone(),
        saved_off: saved_off.clone(),
    };
    let mut engine = Engine::new(host, FakeOverlay(drawn.clone()));
    engine.tick();
    // The checker is asked for nothing in these tests; the fix is given.
    engine.submitted = true;
    engine.fixes = vec![Fix {
        start: 10,
        end: 15,
        replacement: "Paris".into(),
        kind: Kind::Spelling,
        message: String::new(),
        source: Source::Local,
    }];
    Rig {
        engine,
        drawn,
        pointer,
        text,
        focused,
        elsewhere,
        undos,
        clocks,
        saved_off,
    }
}

fn local_only() -> Config {
    Config {
        ai: false,
        ..Config::default()
    }
}

const WORD: Rect = Rect {
    x: 90.0,
    y: 100.0,
    w: 40.0,
    h: 16.0,
};

#[test]
fn marks_are_drawn_where_the_field_reports_the_word() {
    let mut rig = rig(local_only(), true);
    rig.engine.tick();
    let drawn = rig.drawn.borrow();
    let (area, marks) = drawn.marks.last().expect("marks drawn");
    assert_eq!(*area, FIELD.grow(4.0));
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].rect, WORD);
    assert_eq!(marks[0].kind, Kind::Spelling);
    assert!(!marks[0].highlighted);
}

#[test]
fn an_insertion_is_marked_on_the_character_before_it() {
    let mut rig = rig(local_only(), true);
    rig.engine.fixes[0] = Fix {
        start: 15,
        end: 15,
        replacement: "!".into(),
        ..rig.engine.fixes[0].clone()
    };
    rig.engine.tick();
    let drawn = rig.drawn.borrow();
    let (_, marks) = drawn.marks.last().expect("marks drawn");
    assert_eq!(marks[0].rect.x, 10.0 + 14.0 * 8.0);
    assert_eq!(marks[0].rect.w, 8.0);
}

#[test]
fn hovering_a_word_opens_its_card_and_leaving_the_corridor_closes_it() {
    let mut rig = rig(local_only(), true);
    rig.pointer.set(Point { x: 100.0, y: 108.0 });
    rig.engine.tick();
    assert!(rig.drawn.borrow().card.is_none());

    std::thread::sleep(HOVER + Duration::from_millis(10));
    rig.engine.tick();
    {
        let drawn = rig.drawn.borrow();
        let (index, card, word) = drawn.card.as_ref().expect("card opened");
        assert_eq!(*index, 0);
        assert_eq!(*word, WORD);
        assert_eq!(card.title, "Correct your spelling");
        assert!(card.bits.contains(&Bit::New("Paris".into())));
    }
    assert_eq!(rig.engine.card_for, Some(0));

    // In the band between the word (bottom 116) and the card (top 120).
    rig.pointer.set(Point { x: 100.0, y: 118.0 });
    std::thread::sleep(LEAVE + Duration::from_millis(10));
    rig.engine.tick();
    assert!(rig.drawn.borrow().card.is_some());

    rig.pointer.set(Point { x: 900.0, y: 900.0 });
    rig.engine.tick();
    std::thread::sleep(LEAVE + Duration::from_millis(10));
    rig.engine.tick();
    assert!(rig.drawn.borrow().card.is_none());
    assert_eq!(rig.engine.card_for, None);
}

#[test]
fn accepting_from_the_card_writes_the_fix_into_the_field() {
    let mut rig = rig(local_only(), true);
    rig.pointer.set(Point { x: 100.0, y: 108.0 });
    rig.engine.tick();
    std::thread::sleep(HOVER + Duration::from_millis(10));
    rig.engine.tick();

    rig.engine.act(Action::Accept, 0);

    assert_eq!(*rig.text.borrow(), "i went to Paris");
    assert!(rig.drawn.borrow().card.is_none());
    assert_eq!(
        rig.engine
            .expecting
            .as_ref()
            .map(|(_, after, _)| after.as_str()),
        Some("i went to Paris")
    );
}

#[test]
fn dismissing_hides_the_fix_for_good() {
    let mut rig = rig(local_only(), true);
    rig.pointer.set(Point { x: 100.0, y: 108.0 });
    rig.engine.tick();
    std::thread::sleep(HOVER + Duration::from_millis(10));
    rig.engine.tick();
    rig.engine.local = rig.engine.fixes.clone();

    rig.engine.act(Action::Dismiss, 0);

    assert!(rig.engine.fixes.is_empty());
    assert_eq!(*rig.text.borrow(), TEXT);
}

#[test]
fn only_a_refusal_the_writer_can_end_shows_while_ai_is_on() {
    let mut rig = rig(local_only(), true);
    let credits = "OpenRouter refused the text (402): This request requires more credits";
    rig.engine.ai_error = Some(credits.into());
    assert_eq!(rig.engine.status().blocked, None, "AI is off");
    rig.engine.config = Config {
        openrouter_key: Some("key".into()),
        ..Config::default()
    };
    assert_eq!(rig.engine.status().blocked, Some(Blocked::Credits));
    rig.engine.ai_error = Some("could not reach OpenRouter: timed out".into());
    assert_eq!(rig.engine.status().blocked, None);
}

#[test]
fn a_pause_says_until_when_and_resume_ends_it() {
    let mut rig = rig(local_only(), true);
    assert_eq!(rig.engine.status().paused, None);
    rig.engine.pause_for(0);
    assert_eq!(
        rig.engine.status().paused.as_deref(),
        Some("Paused until 3:00 PM")
    );
    rig.engine.tick();
    assert!(rig.engine.field.is_none(), "nothing is read while paused");
    rig.engine.resume();
    assert_eq!(rig.engine.status().paused, None);
    rig.engine.tick();
    assert!(rig.engine.field.is_some());
}

#[test]
fn until_tomorrow_names_no_clock_time() {
    let mut rig = rig(local_only(), true);
    rig.engine.pause_for(PAUSES.len() - 1);
    assert_eq!(
        rig.engine.status().paused.as_deref(),
        Some("Paused until tomorrow")
    );
    assert_eq!(rig.clocks.get(), 0);
}

#[test]
fn the_pause_clock_is_written_once_however_often_the_status_is_read() {
    let mut rig = rig(local_only(), true);
    rig.engine.act(Action::PauseHour, 0);
    for _ in 0..5 {
        rig.engine.status();
    }
    assert_eq!(rig.clocks.get(), 1);
    assert!(rig.engine.status().paused.is_some());
}

#[test]
fn a_field_turned_off_from_the_card_is_saved_and_shows_nothing() {
    let mut rig = rig(local_only(), true);
    rig.engine.act(Action::OffHere, 0);
    assert_eq!(rig.saved_off.borrow().len(), 1);
    assert_eq!(rig.engine.status().off, ["App · Message"]);
    let drawn = rig.drawn.borrow().marks.len();
    for _ in 0..3 {
        rig.engine.tick();
    }
    assert!(rig.engine.field.is_some(), "kept, so it is not asked again");
    assert!(rig.engine.text.is_empty(), "and not read");
    assert_eq!(rig.drawn.borrow().marks.len(), drawn, "nor drawn");

    rig.engine.turn_on(0);
    assert!(rig.saved_off.borrow().is_empty());
    assert!(rig.engine.status().off.is_empty());
    rig.engine.tick();
    assert_eq!(*rig.engine.text, *TEXT);
}

#[test]
fn a_field_with_nothing_to_know_it_by_is_off_until_gramly_quits() {
    let mut rig = rig_labelled(local_only(), true, "");
    rig.engine.act(Action::OffHere, 0);
    assert!(rig.saved_off.borrow().is_empty(), "not saved");
    assert_eq!(rig.engine.status().off, ["App · Text field"]);
    rig.engine.tick();
    assert!(rig.engine.text.is_empty());
    rig.engine.turn_all_on();
    rig.engine.tick();
    assert_eq!(*rig.engine.text, *TEXT);
}

#[test]
fn the_more_button_opens_the_menu() {
    let mut rig = rig(local_only(), true);
    rig.engine.act(Action::More, 0);
    assert_eq!(rig.drawn.borrow().menu_toggles, 1);
}

#[test]
fn an_unchanged_status_compares_equal_so_the_menu_is_not_redrawn() {
    let rig = rig(local_only(), true);
    assert_eq!(rig.engine.status(), rig.engine.status());
}

#[test]
fn a_word_scrolled_out_of_the_field_is_not_in_view() {
    let field = Rect {
        x: 0.0,
        y: 100.0,
        w: 300.0,
        h: 50.0,
    };
    let word = |y| Rect {
        x: 10.0,
        y,
        w: 40.0,
        h: 16.0,
    };
    assert!(scrolled_into(word(100.0), field));
    assert!(scrolled_into(word(98.0), field));
    assert!(scrolled_into(word(136.0), field));
    assert!(!scrolled_into(word(90.0), field));
    assert!(!scrolled_into(word(140.0), field));
}

#[test]
fn a_dismissal_is_keyed_by_the_words_before_it_on_its_line() {
    let fix = |start, end| Fix {
        start,
        end,
        replacement: "OK".into(),
        kind: Kind::Correctness,
        message: String::new(),
        source: Source::Local,
    };
    let text = "so ok then\nwell ok";
    assert_eq!(
        dismissal(text, &fix(3, 5)),
        ("so".into(), "ok".into(), "OK".into())
    );
    assert_eq!(
        dismissal(text, &fix(16, 18)),
        ("well".into(), "ok".into(), "OK".into())
    );
}

const ON_WORD: Point = Point { x: 100.0, y: 108.0 };
const ON_CARD: Point = Point { x: 100.0, y: 150.0 };
const AWAY: Point = Point { x: 900.0, y: 900.0 };

fn open_card(rig: &mut Rig) {
    rig.pointer.set(ON_WORD);
    rig.engine.tick();
    std::thread::sleep(HOVER + Duration::from_millis(10));
    rig.engine.tick();
    assert!(rig.drawn.borrow().card.is_some(), "the card did not open");
    rig.pointer.set(ON_CARD);
}

fn shown_on_card(rig: &Rig) -> String {
    let drawn = rig.drawn.borrow();
    let (_, card, _) = drawn.card.as_ref().expect("a card");
    card.bits
        .iter()
        .find_map(|bit| match bit {
            Bit::New(text) => Some(text.clone()),
            _ => None,
        })
        .expect("a new word on the card")
}

#[test]
fn a_field_that_stops_answering_is_kept_for_seven_ticks_and_dropped_on_the_eighth() {
    let mut rig = rig(local_only(), true);
    let hidden = rig.drawn.borrow().hidden_marks;
    rig.focused.set(false);

    for _ in 0..GRACE - 1 {
        rig.engine.tick();
    }
    assert!(rig.engine.field.is_some());
    assert_eq!(rig.drawn.borrow().hidden_marks, hidden);

    rig.engine.tick();
    assert!(rig.engine.field.is_none());
    assert_eq!(rig.drawn.borrow().hidden_marks, hidden + 1);
}

#[test]
fn a_card_whose_fix_is_withdrawn_stays_until_the_pointer_has_left_for_a_while() {
    let mut rig = rig(local_only(), true);
    open_card(&mut rig);

    rig.engine.local.clear();
    rig.engine.ai.clear();
    rig.engine.refilter();
    assert_eq!(rig.engine.card_for, Some(ORPHAN));

    rig.engine.tick();
    std::thread::sleep(LEAVE + Duration::from_millis(10));
    rig.engine.tick();
    assert!(rig.drawn.borrow().card.is_some());

    rig.pointer.set(AWAY);
    rig.engine.tick();
    assert!(rig.drawn.borrow().card.is_some());
    std::thread::sleep(LEAVE + Duration::from_millis(10));
    rig.engine.tick();
    assert!(rig.drawn.borrow().card.is_none());
    assert_eq!(rig.engine.card_for, None);
}

#[test]
fn a_card_is_untouched_when_the_rebuilt_list_still_has_its_fix() {
    let mut rig = rig(local_only(), true);
    open_card(&mut rig);
    rig.engine.local = rig.engine.fixes.clone();
    let shown = rig.drawn.borrow().cards_shown;

    rig.engine.refilter();
    rig.engine.tick();

    assert_eq!(rig.engine.card_for, Some(0));
    assert_eq!(rig.engine.card_stale, None);
    assert_eq!(rig.drawn.borrow().cards_shown, shown);
}

#[test]
fn a_card_is_redrawn_in_place_when_the_same_words_get_a_new_replacement() {
    let mut rig = rig(local_only(), true);
    open_card(&mut rig);
    rig.engine.local = vec![Fix {
        replacement: "PARIS".into(),
        ..rig.engine.fixes[0].clone()
    }];

    rig.engine.refilter();
    assert_eq!(rig.engine.card_for, Some(0));
    assert_eq!(rig.engine.card_stale, Some(WORD));
    assert!(rig.drawn.borrow().card.is_some());
    assert_eq!(shown_on_card(&rig), "Paris");

    rig.engine.tick();
    assert_eq!(shown_on_card(&rig), "PARIS");
    assert_eq!(rig.engine.card_stale, None);
}

#[test]
fn a_fix_that_eats_the_text_is_undone_once_the_app_has_settled() {
    let mut rig = rig(local_only(), true);
    open_card(&mut rig);
    rig.engine.act(Action::Accept, 0);
    assert_eq!(*rig.text.borrow(), "i went to Paris");

    *rig.text.borrow_mut() = "ok".into();
    rig.engine.tick();
    assert_eq!(rig.undos.get(), 0);

    std::thread::sleep(Duration::from_millis(260));
    rig.engine.tick();
    assert_eq!(rig.undos.get(), 1);
    rig.engine.tick();
    assert_eq!(rig.undos.get(), 1);
}

#[test]
fn harper_is_drawn_when_the_model_has_failed() {
    let keyed = Config {
        openrouter_key: Some("key".into()),
        ..Config::default()
    };
    let drawn_marks = |error: Option<&str>| {
        let mut rig = rig(keyed.clone(), true);
        rig.engine.local = std::mem::take(&mut rig.engine.fixes);
        rig.engine.ai_error = error.map(String::from);
        rig.engine.tick();
        let drawn = rig.drawn.borrow();
        drawn.marks.last().map_or(0, |(_, marks)| marks.len())
    };

    assert_eq!(drawn_marks(None), 0);
    assert_eq!(drawn_marks(Some("timeout")), 1);
}

#[test]
fn adding_a_word_to_the_dictionary_checks_the_text_again() {
    let config_home = std::env::temp_dir().join(format!("gramly-engine-{}", std::process::id()));
    // The dictionary is a file under the config directory; a test must not
    // write into the real one. Set before any engine thread reads it.
    static ISOLATE: std::sync::Once = std::sync::Once::new();
    ISOLATE.call_once(|| unsafe { std::env::set_var("XDG_CONFIG_HOME", &config_home) });

    let mut rig = rig(local_only(), true);
    open_card(&mut rig);
    rig.engine.local = rig.engine.fixes.clone();
    let submitted_before = rig.engine.submitted_at;
    assert!(rig.engine.submitted);

    rig.engine.act(Action::Learn, 0);
    assert!(!rig.engine.submitted);

    rig.engine.tick();
    assert!(rig.engine.submitted);
    assert!(rig.engine.submitted_at > submitted_before);
}

#[test]
fn a_second_accept_is_ignored_while_the_first_is_pending() {
    let mut rig = rig(local_only(), true);
    open_card(&mut rig);
    let fix = rig.engine.fixes[0].clone();
    rig.engine.act(Action::Accept, 0);
    assert_eq!(*rig.text.borrow(), "i went to Paris");

    *rig.text.borrow_mut() = TEXT.into();
    rig.engine.accept_fix(&fix);

    assert_eq!(*rig.text.borrow(), TEXT);
}

#[test]
fn an_accept_is_pending_for_a_second() {
    let now = Instant::now();
    assert!(!accept::accept_pending(None, now));
    assert!(accept::accept_pending(Some(now), now));
    assert!(accept::accept_pending(
        Some(now - Duration::from_millis(900)),
        now
    ));
    assert!(!accept::accept_pending(
        Some(now - Duration::from_secs(1)),
        now
    ));
}

#[test]
fn an_orphaned_card_is_not_accepted_at_offsets_that_moved() {
    let mut rig = rig(local_only(), true);
    open_card(&mut rig);
    rig.engine.local.clear();
    rig.engine.ai.clear();
    rig.engine.refilter();
    assert_eq!(rig.engine.card_for, Some(ORPHAN));

    // A letter typed at the word's edge, before the engine has polled.
    let moved = "i went to xparis";
    *rig.text.borrow_mut() = moved.into();
    rig.engine.text = moved.into();
    rig.engine.act(Action::Accept, 0);

    assert_eq!(*rig.text.borrow(), moved);
    assert!(rig.engine.expecting.is_none());
}

#[test]
fn the_words_a_card_shows_are_the_fixs_words_at_the_offsets() {
    let fix = rig(local_only(), true).engine.fixes[0].clone();
    assert!(!accept::words_moved(TEXT, &fix, "paris"));
    assert!(accept::words_moved("i went to xparis", &fix, "paris"));
}

#[test]
fn an_accepted_fix_is_not_offered_again_before_the_app_shows_the_change() {
    let mut rig = rig(local_only(), true);
    open_card(&mut rig);
    rig.engine.local = rig.engine.fixes.clone();
    let fix = rig.engine.fixes[0].clone();

    rig.engine.act(Action::Accept, 0);
    assert!(rig.engine.fixes.is_empty());

    // The app still reports the old text, and the checker the old fix.
    *rig.text.borrow_mut() = TEXT.into();
    rig.engine.refilter();
    assert!(rig.engine.fixes.is_empty());
    assert!(rig.engine.was_accepted(&fix));
}

#[test]
fn an_accepted_fix_is_not_written_again() {
    let mut rig = rig(local_only(), true);
    let fix = rig.engine.fixes[0].clone();
    rig.engine.accept_fix(&fix);
    assert_eq!(*rig.text.borrow(), "i went to Paris");

    *rig.text.borrow_mut() = TEXT.into();
    rig.engine.expecting = None;
    rig.engine.accept_fix(&fix);

    assert_eq!(*rig.text.borrow(), TEXT);
}

fn improvement() -> Fix {
    Fix {
        start: 0,
        end: 6,
        replacement: "we went".into(),
        kind: Kind::Clarity,
        message: String::new(),
        source: Source::Ai,
    }
}

#[test]
fn an_improvement_waits_for_the_model_and_for_the_corrections_to_be_done() {
    let mut rig = rig(local_only(), true);
    rig.engine.improve = vec![improvement()];
    rig.engine.ai = rig.engine.fixes.clone();
    rig.engine.ai_current = true;

    // "paris" is still to be corrected in this sentence.
    rig.engine.refilter();
    assert!(rig.engine.fixes.iter().all(|fix| fix.kind != Kind::Clarity));

    rig.engine.ai.clear();
    rig.engine.refilter();
    assert_eq!(rig.engine.fixes, [improvement()]);

    rig.engine.ai_current = false;
    rig.engine.refilter();
    assert!(rig.engine.fixes.is_empty());
}

#[test]
fn an_improvement_is_never_grouped_with_corrections() {
    let mut rig = rig(local_only(), true);
    rig.engine.fixes.insert(0, improvement());
    assert_eq!(rig.engine.group(0), [(0, improvement())]);
    assert!(
        rig.engine
            .group(1)
            .iter()
            .all(|(_, f)| f.kind != Kind::Clarity)
    );
}

#[test]
fn an_improvement_is_drawn_under_the_first_words_it_changes() {
    let text = "Can you think for now about it.";
    let fix = Fix {
        start: 8,
        end: 30,
        replacement: "think about it for now".into(),
        kind: Kind::Clarity,
        message: String::new(),
        source: Source::Ai,
    };
    let under = anchor(text, &fix);
    assert!(under.start >= fix.start && under.end < fix.end);
    assert_eq!(fix.original(text), "think for now about it");
}

#[test]
fn focus_going_elsewhere_hides_the_underlines_at_once() {
    let mut rig = rig(local_only(), true);
    rig.engine.tick();
    let hidden = rig.drawn.borrow().hidden_marks;

    rig.elsewhere.set(true);
    rig.engine.tick();

    assert!(rig.engine.field.is_none());
    assert_eq!(rig.drawn.borrow().hidden_marks, hidden + 1);
}
