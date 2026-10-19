//! `gramly lab`: an audit of what accepting a fix does to real text fields.
//!
//! A second process (`gramly lab-host`) opens a window off screen holding
//! the kinds of field Gramly meets: an AppKit text field and text views,
//! one with a bulleted list, and web text boxes (plain, a list, one with an
//! inline chip like ChatGPT's, a textarea). The lab finds them through
//! Accessibility, the way the app finds a focused field, and runs the app's
//! own code on them: where each underline goes (`Field::bounds`) and what
//! accepting writes (`write::one`, `write::many`). The host answers what the
//! field really holds and where a word really is, from AppKit and the DOM,
//! so every check is against the field itself rather than against what
//! Accessibility says.
//!
//! Nothing touches the user's apps: the window never comes forward, and the
//! paste keystroke is sent to the host's process alone (`paste::target`).
//! The clipboard is used, as the app uses it, and put back at the end.

use crate::macos::ax::{self, Field};
use crate::macos::paste;
use crate::surface::Field as _;
use crate::surface::write;
use anyhow::{Context, Result, bail};
use gramly_core::{Fix, Kind, Source};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEventMask, NSScreen,
    NSTextField, NSTextView, NSUserInterfaceItemIdentification, NSView, NSWindow,
    NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSError, NSRange, NSString};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write as _};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// A text, and the fixes to accept in it: each wrong text, in order of
/// appearance, and what it becomes.
struct Case {
    name: &'static str,
    text: &'static str,
    fixes: &'static [(&'static str, &'static str)],
}

const CASES: &[Case] = &[
    Case {
        name: "typo",
        text: "Can you check teh numbers before the meeting",
        fixes: &[("teh", "the")],
    },
    Case {
        name: "split",
        text: "Pick an oldad from the library",
        fixes: &[("oldad", "old ad")],
    },
    Case {
        name: "join",
        text: "Create a draft. Pickan old ad",
        fixes: &[("Pickan", "Pick an")],
    },
    Case {
        name: "three",
        text: "Let me no when your free tommorow",
        fixes: &[("no", "know"), ("your", "you're"), ("tommorow", "tomorrow")],
    },
    Case {
        name: "neighbours",
        text: "Their is a problem with there account",
        fixes: &[("Their", "There"), ("there", "their")],
    },
    Case {
        name: "punctuation",
        text: "Hey yes  was about to say it went live",
        fixes: &[("Hey", "Hey,"), ("yes  was", "yes, I was")],
    },
    Case {
        name: "end",
        text: "what do you think",
        fixes: &[("what", "What"), ("think", "think?")],
    },
    Case {
        name: "emoji",
        text: "🚀 Shiping teh release 🎉 tommorow",
        fixes: &[
            ("Shiping", "Shipping"),
            ("teh", "the"),
            ("tommorow", "tomorrow"),
        ],
    },
    Case {
        name: "french",
        text: "Je suis aller au marché et j'ai acheter du pain",
        fixes: &[("aller", "allé"), ("acheter", "acheté")],
    },
    Case {
        name: "lines",
        text: "Hi team,\nthe deploy is tonight, if anything break we roll back\nThanks",
        fixes: &[("the", "The"), ("break", "breaks,")],
    },
    Case {
        name: "list",
        text: "- fix the llimit\n- add a tset\n- ship it",
        fixes: &[("llimit", "limit"), ("tset", "test")],
    },
    Case {
        name: "list-start",
        text: "- llimit the scope\n- teh rest",
        fixes: &[("llimit", "limit"), ("teh", "the")],
    },
    // An improvement: most of a sentence rewritten, words moved, the end
    // mark changed, with a sentence after it that must not move.
    Case {
        name: "improve",
        text: "Can you think for now about conservation. What can I keep?",
        fixes: &[("for now about conservation.", "about conservation for now?")],
    },
    // Removals: a repeated word, and the second of two commas. In Slack the
    // comma was pasted over as nothing and stayed; a removal is a Delete.
    Case {
        name: "removal",
        text: "Send me the the link,, thanks",
        fixes: &[("the ", ""), (",", "")],
    },
    Case {
        name: "chip",
        text: "Pick {chip} an oldad for teh campaign",
        fixes: &[("oldad", "old ad"), ("teh", "the")],
    },
];

/// The fields the host offers, by the identifier the lab finds them by.
const FIELDS: &[(&str, &str)] = &[
    ("lab-field", "AppKit text field"),
    ("lab-note", "AppKit text view"),
    ("lab-rich", "AppKit rich text, bullets"),
    ("web-plain", "Web text box"),
    ("web-list", "Web list"),
    ("web-chip", "Web text with a chip"),
    ("web-area", "Web textarea"),
];

fn fits(field: &str, case: &Case) -> bool {
    let lines = case.text.contains('\n');
    let list = case.text.starts_with("- ");
    let chip = case.text.contains("{chip}");
    match field {
        "lab-field" => !lines && !chip,
        "web-list" => list,
        "lab-rich" => list,
        "web-chip" => chip,
        _ => !chip,
    }
}

// ---------------------------------------------------------------- the host

const PAGE: &str = r#"<!doctype html><html><body style="font:14px -apple-system;margin:8px">
<div id="web-plain" contenteditable="true" style="white-space:pre-wrap;border:1px solid #ccc;min-height:48px;padding:4px;margin-bottom:6px"></div>
<div id="web-list" contenteditable="true" style="border:1px solid #ccc;min-height:48px;padding:4px;margin-bottom:6px"></div>
<div id="web-chip" contenteditable="true" style="border:1px solid #ccc;min-height:24px;padding:4px;margin-bottom:6px"></div>
<textarea id="web-area" rows="4" cols="70"></textarea>
<script>
function esc(s){return s.replace(/[&<>]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;'}[c]))}
function load(id,text){
  const el=document.getElementById(id);
  if(id==='web-area'){el.value=text;return 'ok'}
  if(id==='web-list'){
    el.innerHTML='<ul>'+text.split('\n').filter(l=>l).map(l=>'<li>'+esc(l.replace(/^- /,''))+'</li>').join('')+'</ul>';
    return 'ok'}
  let html=text.split('\n').map(l=>'<div>'+(l?esc(l):'<br>')+'</div>').join('');
  html=html.replace('{chip}','<span contenteditable="false" style="background:#eee;border-radius:4px;padding:0 4px"><img width="10" height="10" src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7">AdCollection</span>');
  el.innerHTML=html;return 'ok'}
function truth(id){const el=document.getElementById(id);return el.tagName==='TEXTAREA'?el.value:el.innerText}
function focusOn(id){const el=document.getElementById(id);el.focus();return 'ok'}
function rectOf(id,word,nth){
  const el=document.getElementById(id);if(el.tagName==='TEXTAREA')return 'null';
  const w=document.createTreeWalker(el,NodeFilter.SHOW_TEXT);let nodes=[],full='';
  while(w.nextNode()){nodes.push([w.currentNode,full.length]);full+=w.currentNode.data}
  let at=-1;for(let i=0;i<=nth;i++){at=full.indexOf(word,at+1);if(at<0)return 'null'}
  const find=o=>{for(let i=nodes.length-1;i>=0;i--){if(nodes[i][1]<=o)return [nodes[i][0],o-nodes[i][1]]}};
  const [sn,so]=find(at),[en,eo]=find(at+word.length-1);
  const r=document.createRange();r.setStart(sn,so);r.setEnd(en,eo+1);
  const b=r.getClientRects()[0]||r.getBoundingClientRect();
  return JSON.stringify({x:b.x,y:b.y,w:b.width,h:b.height})}
</script></body></html>"#;

struct Host {
    mtm: MainThreadMarker,
    window: Retained<NSWindow>,
    field: Retained<NSTextField>,
    note: Retained<NSTextView>,
    rich: Retained<NSTextView>,
    web: Retained<WKWebView>,
}

/// Off every screen, so the window is real to Accessibility and AppKit and
/// never in the user's way.
const OFF_SCREEN: CGPoint = CGPoint {
    x: -6000.0,
    y: -6000.0,
};

pub fn host() -> Result<()> {
    let mtm = MainThreadMarker::new().context("not on the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    // ⌘V is a menu key equivalent; without an Edit menu nothing pastes.
    app.setMainMenu(Some(&crate::macos::app::edit_menu(mtm)));
    #[allow(deprecated)]
    app.finishLaunching();

    let size = CGSize::new(900.0, 720.0);
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            CGRect::new(OFF_SCREEN, size),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    let content = NSView::initWithFrame(
        NSView::alloc(mtm),
        CGRect::new(CGPoint::new(0.0, 0.0), size),
    );

    let field = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        CGRect::new(CGPoint::new(10.0, 680.0), CGSize::new(880.0, 24.0)),
    );
    field.setIdentifier(Some(&NSString::from_str("lab-field")));
    content.addSubview(&field);

    let text_view = |id: &str, x: f64, rich: bool| {
        let scroll = NSTextView::scrollableTextView(mtm);
        scroll.setFrame(CGRect::new(
            CGPoint::new(x, 450.0),
            CGSize::new(435.0, 220.0),
        ));
        let view: Retained<NSTextView> = scroll
            .documentView()
            .and_then(|v| v.downcast::<NSTextView>().ok())
            .expect("a scrollable text view holds a text view");
        view.setRichText(rich);
        view.setIdentifier(Some(&NSString::from_str(id)));
        content.addSubview(&scroll);
        view
    };
    let note = text_view("lab-note", 10.0, false);
    let rich = text_view("lab-rich", 455.0, true);

    let web = unsafe {
        WKWebView::initWithFrame_configuration(
            WKWebView::alloc(mtm),
            CGRect::new(CGPoint::new(10.0, 10.0), CGSize::new(880.0, 430.0)),
            &WKWebViewConfiguration::new(mtm),
        )
    };
    unsafe { web.loadHTMLString_baseURL(&NSString::from_str(PAGE), None) };
    content.addSubview(&web);

    window.setContentView(Some(&content));
    window.orderFrontRegardless();

    let host = Host {
        mtm,
        window,
        field,
        note,
        rich,
        web,
    };
    // The page loads asynchronously; it is ready when its script answers.
    let started = Instant::now();
    while host.js("typeof load").as_deref() != Some("function") {
        if started.elapsed() > Duration::from_secs(10) {
            bail!("the test page did not load");
        }
        host.spin(0.05);
    }

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    answer(&json!({"ready": true, "pid": std::process::id()}));
    loop {
        match rx.try_recv() {
            Ok(line) => {
                let request: Value = serde_json::from_str(&line)?;
                answer(&host.handle(&request));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => host.spin(0.01),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
        }
    }
}

fn answer(value: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

fn js_string(s: &str) -> String {
    serde_json::to_string(s).expect("a string serialises")
}

impl Host {
    /// Runs the app for `seconds`, delivering events: the posted ⌘V among
    /// them. An app in the background has no key window, so a ⌘V sent to
    /// it is handed to the window's first responder here, as the Edit menu
    /// would hand it to the key window's.
    fn spin(&self, seconds: f64) {
        let app = NSApplication::sharedApplication(self.mtm);
        let until = NSDate::dateWithTimeIntervalSinceNow(seconds);
        while let Some(event) = unsafe {
            app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&until),
                NSDefaultRunLoopMode,
                true,
            )
        } {
            let command = event
                .modifierFlags()
                .contains(objc2_app_kit::NSEventModifierFlags::Command);
            let key = (event.r#type() == objc2_app_kit::NSEventType::KeyDown)
                .then(|| event.charactersIgnoringModifiers())
                .flatten()
                .map(|c| c.to_string());
            let shift = event
                .modifierFlags()
                .contains(objc2_app_kit::NSEventModifierFlags::Shift);
            let arrow =
                (event.r#type() == objc2_app_kit::NSEventType::KeyDown).then(|| event.keyCode());
            match (command, key.as_deref(), app.keyWindow().is_none()) {
                (true, Some("v"), true) => self.perform(sel!(paste:)),
                (true, Some("z"), true) => self.perform(sel!(undo:)),
                (false, _, true) if shift && arrow == Some(124) => {
                    self.perform(sel!(moveRightAndModifySelection:))
                }
                (false, _, true) if !shift && arrow == Some(123) => self.perform(sel!(moveLeft:)),
                _ => app.sendEvent(&event),
            }
        }
    }

    fn perform(&self, action: objc2::runtime::Sel) {
        if let Some(responder) = self.window.firstResponder() {
            let _ = unsafe { responder.tryToPerform_with(action, None) };
        }
    }

    /// Evaluates `script` in the page and waits for its string answer.
    fn js(&self, script: &str) -> Option<String> {
        let result: Rc<RefCell<Option<Option<String>>>> = Rc::new(RefCell::new(None));
        let sink = result.clone();
        let block = block2::RcBlock::new(move |value: *mut AnyObject, _error: *mut NSError| {
            let text = unsafe { value.as_ref() }
                .and_then(|v| v.downcast_ref::<NSString>())
                .map(|s| s.to_string());
            *sink.borrow_mut() = Some(text);
        });
        unsafe {
            self.web
                .evaluateJavaScript_completionHandler(&NSString::from_str(script), Some(&block))
        };
        let started = Instant::now();
        while result.borrow().is_none() && started.elapsed() < Duration::from_secs(5) {
            self.spin(0.005);
        }
        result.borrow_mut().take().flatten()
    }

    fn text_view(&self, id: &str) -> Option<&NSTextView> {
        match id {
            "lab-note" => Some(&self.note),
            "lab-rich" => Some(&self.rich),
            _ => None,
        }
    }

    fn handle(&self, request: &Value) -> Value {
        let id = request["field"].as_str().unwrap_or_default();
        match request["cmd"].as_str().unwrap_or_default() {
            "load" => {
                let text = request["text"].as_str().unwrap_or_default();
                match id {
                    "lab-field" => self.field.setStringValue(&NSString::from_str(text)),
                    // A bulleted list as TextEdit writes one: the marker is
                    // text, between tabs.
                    "lab-rich" => {
                        let listed: Vec<String> = text
                            .lines()
                            .map(|l| match l.strip_prefix("- ") {
                                Some(item) => format!("\t•\t{item}"),
                                None => l.to_string(),
                            })
                            .collect();
                        self.rich.setString(&NSString::from_str(&listed.join("\n")));
                    }
                    "lab-note" => self.note.setString(&NSString::from_str(text)),
                    _ => {
                        self.js(&format!("load({},{})", js_string(id), js_string(text)));
                    }
                }
                self.focus(id);
                self.spin(0.15);
                json!({"ok": true})
            }
            "truth" => json!({"text": self.truth(id)}),
            "rect" => {
                let word = request["word"].as_str().unwrap_or_default();
                let nth = request["nth"].as_u64().unwrap_or(0) as usize;
                json!({"rect": self.rect(id, word, nth)})
            }
            "spin" => {
                self.spin(request["seconds"].as_f64().unwrap_or(0.1));
                json!({"ok": true})
            }
            other => json!({"error": format!("unknown command {other:?}")}),
        }
    }

    fn focus(&self, id: &str) {
        match id {
            "lab-field" => {
                self.window.makeFirstResponder(Some(&self.field));
            }
            "lab-note" | "lab-rich" => {
                if let Some(view) = self.text_view(id) {
                    self.window.makeFirstResponder(Some(view));
                }
            }
            _ => {
                self.window.makeFirstResponder(Some(&self.web));
                self.js(&format!("focusOn({})", js_string(id)));
            }
        }
    }

    fn truth(&self, id: &str) -> String {
        match id {
            "lab-field" => match self.field.currentEditor() {
                Some(editor) => editor.string().to_string(),
                None => self.field.stringValue().to_string(),
            },
            "lab-note" | "lab-rich" => self
                .text_view(id)
                .map(|v| v.string().to_string())
                .unwrap_or_default(),
            _ => self
                .js(&format!("truth({})", js_string(id)))
                .unwrap_or_default(),
        }
    }

    /// Where the `nth` occurrence of `word` is drawn, in Accessibility's
    /// coordinates (global, top-left origin, y down), as AppKit or the DOM
    /// lays it out.
    fn rect(&self, id: &str, word: &str, nth: usize) -> Option<[f64; 4]> {
        let primary = NSScreen::screens(self.mtm)
            .firstObject()?
            .frame()
            .size
            .height;
        let flip = |r: CGRect| {
            [
                r.origin.x,
                primary - r.origin.y - r.size.height,
                r.size.width,
                r.size.height,
            ]
        };
        if let Some(view) = self.text_view(id) {
            let text = view.string().to_string();
            let at = text.match_indices(word).nth(nth)?.0;
            let start = text[..at].encode_utf16().count();
            let range = NSRange::new(start, word.encode_utf16().count());
            let layout = unsafe { view.layoutManager()? };
            let container = unsafe { view.textContainer()? };
            let glyphs = unsafe {
                layout.glyphRangeForCharacterRange_actualCharacterRange(range, std::ptr::null_mut())
            };
            let mut rect = layout.boundingRectForGlyphRange_inTextContainer(glyphs, &container);
            let origin = view.textContainerOrigin();
            rect.origin.x += origin.x;
            rect.origin.y += origin.y;
            let in_window = view.convertRect_toView(rect, None);
            return Some(flip(self.window.convertRectToScreen(in_window)));
        }
        if id.starts_with("web-") {
            let answer = self.js(&format!(
                "rectOf({},{},{nth})",
                js_string(id),
                js_string(word)
            ))?;
            let r: Value = serde_json::from_str(&answer).ok()?;
            let (x, y, w, h) = (
                r["x"].as_f64()?,
                r["y"].as_f64()?,
                r["w"].as_f64()?,
                r["h"].as_f64()?,
            );
            let frame = self
                .window
                .convertRectToScreen(self.web.convertRect_toView(self.web.bounds(), None));
            let top = frame.origin.y + frame.size.height;
            return Some([frame.origin.x + x, primary - top + y, w, h]);
        }
        None
    }
}

// -------------------------------------------------------------- the driver

/// Where the fields live: the off-screen window, or a test page in a
/// browser. Both answer the same requests: `load`, `truth`, `rect`, `spin`.
trait Surface {
    fn ask(&mut self, request: Value) -> Result<Value>;
    /// The field with this id, found through Accessibility.
    fn find(&self, id: &str) -> Option<Field>;

    /// Whether keystrokes can reach the field. A browser tab only takes a
    /// paste while its window is in front; the off-screen window always.
    fn in_front(&self) -> bool {
        true
    }

    /// Fails when the page does not have the keyboard on `id`: a paste
    /// would go somewhere else.
    fn check_focus(&mut self, _id: &str) -> Result<()> {
        Ok(())
    }

    fn truth(&mut self, field: &str) -> Result<String> {
        let answer = self.ask(json!({"cmd": "truth", "field": field}))?;
        Ok(answer["text"].as_str().unwrap_or_default().to_string())
    }
}

/// The off-screen window (`lab-host`), over its stdin and stdout.
struct Window {
    pid: i32,
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Window {
    fn open() -> Result<Self> {
        let mut child = Command::new(std::env::current_exe()?)
            .arg("lab-host")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let input = child.stdin.take().context("host stdin")?;
        let mut output = BufReader::new(child.stdout.take().context("host stdout")?);
        let mut ready = String::new();
        output.read_line(&mut ready)?;
        let ready: Value = serde_json::from_str(&ready).context("the test window did not start")?;
        let pid = ready["pid"].as_i64().context("no pid")? as i32;
        Ok(Self {
            pid,
            child,
            input,
            output,
        })
    }
}

impl Surface for Window {
    fn ask(&mut self, request: Value) -> Result<Value> {
        writeln!(self.input, "{request}")?;
        self.input.flush()?;
        let mut line = String::new();
        if self.output.read_line(&mut line)? == 0 {
            bail!("the test window closed");
        }
        Ok(serde_json::from_str(&line)?)
    }

    /// WebKit rebuilds its tree after a page change; a field missing for a
    /// moment is looked for again, as in a browser.
    fn find(&self, id: &str) -> Option<Field> {
        let started = Instant::now();
        loop {
            if let Some(field) = ax::find(self.pid, None, id) {
                return Some(field);
            }
            if started.elapsed() > Duration::from_secs(5) {
                return None;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// The browsers the lab can open its page in: Chrome's engine is also
/// Slack's, Claude's and ChatGPT's (Electron); Zen's is Firefox's.
const BROWSERS: &[(&str, &str, &str)] = &[
    ("chrome", "Google Chrome", "com.google.Chrome"),
    ("zen", "Zen", "app.zen-browser.zen"),
    ("safari", "Safari", "com.apple.Safari"),
    ("firefox", "Firefox", "org.mozilla.firefox"),
];

const TITLE: &str = "Gramly lab";

/// The page talks to the lab over plain HTTP on 127.0.0.1: it asks for the
/// next request, runs it, and posts the answer back. Any browser can run
/// that, and it gives the lab the DOM's own account of every field.
const BRIDGE: &str = r#"<script>
document.title='Gramly lab';
function run(c){
  if(c.cmd==='ping')return {ok:true};
  if(c.cmd==='load'){load(c.field,c.text);focusOn(c.field);return {ok:true}}
  if(c.cmd==='focus')return {page:document.hasFocus(),active:(document.activeElement||{}).id||''};
  if(c.cmd==='truth')return {text:truth(c.field)};
  if(c.cmd==='rect'){
    const w=rectOf(c.field,c.word,c.nth);
    const f=document.getElementById(c.field).getBoundingClientRect();
    return {word:w==='null'?null:JSON.parse(w),field:{x:f.x,y:f.y,w:f.width,h:f.height}}}
  if(c.cmd==='sel'){const s=getSelection();const d=n=>n?(n.nodeType===3?JSON.stringify(n.data):n.nodeName+'['+n.childNodes.length+']'):'-';
    return {text:JSON.stringify(s.toString()),anchor:d(s.anchorNode)+'@'+s.anchorOffset,focus:d(s.focusNode)+'@'+s.focusOffset}}
  if(c.cmd==='done'){document.body.innerHTML='<p style="font:16px -apple-system;margin:40px">Gramly lab finished. You can close this tab.</p>';return {ok:true}}
  return {error:'unknown '+c.cmd}}
(async function(){for(;;){try{
  const r=await fetch('/next',{cache:'no-store'});
  if(r.status!==200)continue;
  const c=await r.json();let out;
  try{out=run(c)}catch(e){out={error:String(e)}}
  await fetch('/reply',{method:'POST',body:JSON.stringify(out)});
}catch(e){await new Promise(r=>setTimeout(r,200))}}})();
</script>"#;

struct Browser {
    pid: i32,
    bundle: String,
    commands: std::sync::mpsc::Sender<String>,
    replies: std::sync::mpsc::Receiver<String>,
}

impl Browser {
    /// Serves the page, opens it in a new tab of `app`, and waits for it.
    fn open(app: &str, bundle: &str) -> Result<Self> {
        use std::sync::{Arc, Mutex, mpsc};
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let page = format!("{}{BRIDGE}", PAGE.replace("</body></html>", ""));
        let (commands, waiting) = mpsc::channel::<String>();
        let waiting = Arc::new(Mutex::new(waiting));
        let (answered, replies) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (page, waiting, answered) = (page.clone(), waiting.clone(), answered.clone());
                std::thread::spawn(move || {
                    let _ = serve(stream, &page, &waiting, &answered);
                });
            }
        });
        let status = Command::new("open")
            .args(["-a", app, &format!("http://127.0.0.1:{port}/")])
            .status()?;
        if !status.success() {
            bail!("{app} could not be opened");
        }
        let mut browser = Self {
            pid: 0,
            bundle: bundle.to_string(),
            commands,
            replies,
        };
        browser
            .ask(json!({"cmd": "ping"}))
            .with_context(|| format!("the page did not load in {app}"))?;
        browser.pid = objc2_app_kit::NSRunningApplication::runningApplicationsWithBundleIdentifier(
            &NSString::from_str(bundle),
        )
        .firstObject()
        .map(|a| a.processIdentifier())
        .with_context(|| format!("{app} is not running"))?;
        ax::wake(browser.pid);
        std::thread::sleep(Duration::from_millis(800));
        Ok(browser)
    }
}

/// One HTTP exchange with the page.
fn serve(
    stream: std::net::TcpStream,
    page: &str,
    waiting: &std::sync::Mutex<std::sync::mpsc::Receiver<String>>,
    answered: &std::sync::mpsc::Sender<String>,
) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut first = String::new();
    reader.read_line(&mut first)?;
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        if header.trim().is_empty() {
            break;
        }
        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    std::io::Read::read_exact(&mut reader, &mut body)?;
    let path = first.split_whitespace().nth(1).unwrap_or("/");
    let (status, kind, content) = match path {
        "/next" => {
            let next = waiting
                .lock()
                .map_err(|_| anyhow::anyhow!("lock"))?
                .recv_timeout(Duration::from_secs(20));
            match next {
                Ok(command) => ("200 OK", "application/json", command),
                Err(_) => ("204 No Content", "text/plain", String::new()),
            }
        }
        "/reply" => {
            let _ = answered.send(String::from_utf8_lossy(&body).into_owned());
            ("204 No Content", "text/plain", String::new())
        }
        "/" => ("200 OK", "text/html; charset=utf-8", page.to_string()),
        _ => ("404 Not Found", "text/plain", String::new()),
    };
    let mut out = stream;
    write!(
        out,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{content}",
        content.len()
    )?;
    Ok(())
}

impl Surface for Browser {
    fn ask(&mut self, request: Value) -> Result<Value> {
        if request["cmd"] == "spin" {
            std::thread::sleep(Duration::from_secs_f64(
                request["seconds"].as_f64().unwrap_or(0.1),
            ));
            return Ok(json!({"ok": true}));
        }
        let rect = request["cmd"] == "rect";
        let field = request["field"].as_str().unwrap_or_default().to_string();
        self.commands.send(request.to_string())?;
        let answer: Value = serde_json::from_str(
            &self
                .replies
                .recv_timeout(Duration::from_secs(25))
                .context("the page stopped answering")?,
        )?;
        if let Some(error) = answer["error"].as_str() {
            bail!("the page: {error}");
        }
        if !rect {
            return Ok(answer);
        }
        // The page knows where the word is inside its viewport; where the
        // viewport is on screen comes from the field's own frame.
        let word = &answer["word"];
        let page_field = &answer["field"];
        let frame = self.find(&field).and_then(|f| f.frame());
        let (Some(frame), Some(x), Some(y), Some(w), Some(h), Some(fx), Some(fy)) = (
            frame,
            word["x"].as_f64(),
            word["y"].as_f64(),
            word["w"].as_f64(),
            word["h"].as_f64(),
            page_field["x"].as_f64(),
            page_field["y"].as_f64(),
        ) else {
            return Ok(json!({"rect": null}));
        };
        Ok(json!({"rect": [frame.origin.x + x - fx, frame.origin.y + y - fy, w, h]}))
    }

    /// Chrome builds its accessibility tree a moment after it is asked
    /// to, so a field not there yet is looked for again for a few seconds.
    fn in_front(&self) -> bool {
        ax::frontmost_app().is_some_and(|(id, _)| id == self.bundle)
    }

    fn check_focus(&mut self, id: &str) -> Result<()> {
        let focus = self.ask(json!({"cmd": "focus"}))?;
        if focus["page"] == true && focus["active"] == id {
            return Ok(());
        }
        let window = ax::frontmost_window_title().unwrap_or_default();
        note!("lab: the page does not have the keyboard ({focus}); front window {window:?}");
        bail!(INTERRUPTED);
    }

    fn find(&self, id: &str) -> Option<Field> {
        let started = Instant::now();
        loop {
            if let Some(field) = ax::find(self.pid, Some(TITLE), id) {
                return Some(field);
            }
            if started.elapsed() > Duration::from_secs(5) {
                return None;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

/// What happened to one accept.
#[derive(Debug, Clone, PartialEq)]
enum Result_ {
    /// The field holds exactly what it should.
    Exact,
    /// Gramly refused to write, and why. Safe, but the fix did nothing.
    Refused(String),
    /// Gramly wrote, and the field never changed: the click did nothing.
    Lost,
    /// The field holds something else: the failure that matters.
    Wrong { got: String, want: String },
}

#[derive(Default)]
struct Tally {
    placed: usize,
    misplaced: usize,
    unplaced: usize,
    exact: usize,
    refused: usize,
    lost: usize,
    wrong: usize,
    /// Cases the lab could not run (a field not found, no answer): never a
    /// pass.
    broken: usize,
    millis: Vec<u128>,
    /// How long Accessibility took to report a change the page had made.
    lag: Vec<u128>,
    notes: Vec<String>,
}

/// The words, as a person reads them. The DOM and Accessibility differ on
/// everything else: Chrome reports a list item's "• " and line breaks
/// around an inline chip that the page's text has not, and both differ on
/// no-break spaces and a final line break.
fn plain(text: &str) -> String {
    text.replace(['\u{a0}', '\u{fffc}', '•'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The field's value once Accessibility reports what the page holds, as the
/// app's poll would see it a moment later. None if it never does.
fn settled(field: &Field, truth: &str, tally: &mut Tally) -> Option<String> {
    let started = Instant::now();
    let want = plain(truth);
    loop {
        if let Some(value) = field.value()
            && plain(&value) == want
        {
            tally.lag.push(started.elapsed().as_millis());
            return Some(value);
        }
        if started.elapsed() > Duration::from_secs(3) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The `n`th occurrence of `needle` in `haystack` as a char range.
fn nth_chars(haystack: &str, needle: &str, n: usize) -> Option<(usize, usize)> {
    let (byte, _) = haystack.match_indices(needle).nth(n)?;
    let start = haystack[..byte].chars().count();
    Some((start, start + needle.chars().count()))
}

/// `text` with the `n`th occurrence of `from` replaced by `to`.
fn replace_nth(text: &str, from: &str, to: &str, n: usize) -> Option<String> {
    let (byte, _) = text.match_indices(from).nth(n)?;
    Some(format!(
        "{}{to}{}",
        &text[..byte],
        &text[byte + from.len()..]
    ))
}

fn fix_at(text: &str, from: &str, to: &str, n: usize) -> Option<Fix> {
    let (start, end) = nth_chars(text, from, n)?;
    Some(Fix {
        start,
        end,
        replacement: to.to_string(),
        kind: Kind::Correctness,
        message: String::new(),
        source: Source::Ai,
    })
}

/// Which occurrence of `from` each fix of a case is: "teh" twice in one
/// text is the first, then the second.
fn occurrences(case: &Case) -> Vec<usize> {
    case.fixes
        .iter()
        .enumerate()
        .map(|(i, (from, _))| case.fixes[..i].iter().filter(|(f, _)| f == from).count())
        .collect()
}

pub fn run(args: &[String]) -> Result<()> {
    if !ax::trusted(false) {
        bail!("this terminal needs Accessibility access to run the lab");
    }
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    if args.iter().any(|a| a == "--e2e") {
        return e2e(args);
    }
    let only = flag("--only");
    let browser = flag("--browser");
    let show = args.iter().any(|a| a == "--show");

    let board = objc2_app_kit::NSPasteboard::generalPasteboard();
    let saved = paste::save(&board);
    let mut report = Vec::new();
    let result = (|| -> Result<()> {
        match browser.as_deref() {
            None => {
                let mut window = Window::open()?;
                let pid = window.pid;
                run_on(
                    &mut window,
                    pid,
                    "",
                    FIELDS,
                    only.as_deref(),
                    show,
                    &mut report,
                )
            }
            Some(wanted) => {
                let web: Vec<(&str, &str)> = FIELDS
                    .iter()
                    .copied()
                    .filter(|(id, _)| id.starts_with("web-"))
                    .collect();
                for (name, app, bundle) in BROWSERS {
                    if wanted != "all" && !wanted.split(',').any(|w| w == *name) {
                        continue;
                    }
                    if !std::path::Path::new(&format!("/Applications/{app}.app")).exists() {
                        println!("{app}: not installed, skipped");
                        continue;
                    }
                    let mut tab = match Browser::open(app, bundle) {
                        Ok(tab) => tab,
                        Err(err) => {
                            println!("{app}: {err:#}");
                            continue;
                        }
                    };
                    let pid = tab.pid;
                    if args.iter().any(|a| a == "--probe") {
                        probe(&mut tab)?;
                        let _ = tab.ask(json!({"cmd": "done"}));
                        continue;
                    }
                    if run_on(&mut tab, pid, app, &web, only.as_deref(), show, &mut report).is_err()
                    {
                        continue;
                    }
                    let _ = tab.ask(json!({"cmd": "done"}));
                }
                Ok(())
            }
        }
    })();

    // Give the last paste's own restore time to run, then put back what
    // the clipboard held before the lab.
    std::thread::sleep(Duration::from_millis(400));
    paste::restore(&board, saved);
    paste::target(0);
    result?;

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/lab");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("last.json");
    std::fs::write(&path, serde_json::to_string_pretty(&json!(report))?)?;
    println!(
        "\nplaced: underlines within 3pt of the word. exact: accepts that left exactly the\n\
         corrected text. refused: Gramly wrote nothing (safe, but the fix did nothing).\n\
         shown: median time from the click until the field showed the fix.\n\
         ax lag: 90th percentile wait for Accessibility to report what the field shows.\n\
         lost: Gramly wrote and the field never changed. WRONG: the field ended with\n\
         other text - the failure that must stay at 0.\n\
         Every accept is listed with --show. Report: {}",
        path.display()
    );
    Ok(())
}

/// Every case of every field on one surface, one table row per field.
fn run_on(
    surface: &mut dyn Surface,
    pid: i32,
    app: &str,
    fields: &[(&str, &str)],
    only: Option<&str>,
    show: bool,
    report: &mut Vec<Value>,
) -> Result<()> {
    // Keystrokes to this surface's process only.
    paste::target(pid);
    if !app.is_empty() {
        println!("\n{app}");
    }
    println!(
        "{:<28} {:>5} {:>8} {:>7} {:>8} {:>5} {:>6} {:>6} {:>6}",
        "field", "cases", "placed", "exact", "refused", "lost", "WRONG", "shown", "ax lag"
    );
    for (id, label) in fields {
        if only.is_some_and(|o| !id.contains(o)) {
            continue;
        }
        let mut tally = Tally::default();
        let cases: Vec<&Case> = CASES.iter().filter(|c| fits(id, c)).collect();
        for case in &cases {
            if let Err(err) = audit(surface, id, case, &mut tally) {
                if err.to_string() == INTERRUPTED {
                    println!("{label}: stopped, {INTERRUPTED}. Run again with the Mac left alone.");
                    return Err(err);
                }
                tally.broken += 1;
                tally.notes.push(format!("{}: {err:#}", case.name));
            }
        }
        tally.millis.sort_unstable();
        tally.lag.sort_unstable();
        let lag = tally
            .lag
            .get(tally.lag.len() * 9 / 10)
            .copied()
            .unwrap_or(0);
        let p50 = tally
            .millis
            .get(tally.millis.len() / 2)
            .copied()
            .unwrap_or(0);
        let accepts = tally.exact + tally.refused + tally.lost + tally.wrong;
        println!(
            "{:<28} {:>5} {:>4}/{:<3} {:>3}/{:<3} {:>8} {:>5} {:>6} {:>4}ms {:>4}ms",
            label,
            cases.len(),
            tally.placed,
            tally.placed + tally.misplaced + tally.unplaced,
            tally.exact,
            accepts,
            tally.refused,
            tally.lost,
            tally.wrong,
            p50,
            lag
        );
        if show || tally.wrong + tally.refused + tally.lost + tally.misplaced + tally.broken > 0 {
            for note in &tally.notes {
                if show || !note.ends_with(": exact") {
                    println!("    {note}");
                }
            }
        }
        report.push(json!({
            "app": if app.is_empty() { "lab window" } else { app },
            "field": id, "label": label, "cases": cases.len(),
            "placed": tally.placed, "misplaced": tally.misplaced, "unplaced": tally.unplaced,
            "exact": tally.exact, "refused": tally.refused, "lost": tally.lost, "wrong": tally.wrong,
            "broken": tally.broken,
            "p50_ms": p50, "ax_lag_p90_ms": lag, "notes": tally.notes,
        }));
    }
    Ok(())
}

/// The run cannot go on: the test tab is not in front.
const INTERRUPTED: &str = "the test tab left the front (another app or tab was opened)";

fn audit(lab: &mut dyn Surface, id: &str, case: &Case, tally: &mut Tally) -> Result<()> {
    let nths = occurrences(case);
    if !lab.in_front() {
        bail!(INTERRUPTED);
    }
    lab.check_focus(id)?;

    // Placement, on the text as loaded.
    lab.ask(json!({"cmd": "load", "field": id, "text": case.text}))?;
    let field = lab.find(id).with_context(|| format!("{id} not found"))?;
    let loaded = lab.truth(id)?;
    let Some(value) = settled(&field, &loaded, tally) else {
        tally.notes.push(format!(
            "{}: Accessibility never reported the loaded text ({:?})",
            case.name,
            field.value()
        ));
        return Ok(());
    };
    for ((from, _), &n) in case.fixes.iter().zip(&nths) {
        let Some(fix) = fix_at(&value, from, "", n) else {
            tally.notes.push(format!(
                "{}: {from:?} is not in the value {value:?}",
                case.name
            ));
            continue;
        };
        let (at, len) = fix.utf16(&value);
        let drawn = field.bounds_utf16(at, len);
        let truth = lab.ask(json!({"cmd": "rect", "field": id, "word": from, "nth": n}))?;
        let truth: Option<Vec<f64>> = serde_json::from_value(truth["rect"].clone()).ok();
        match (drawn, truth) {
            (Some(d), Some(t)) => {
                let dx = (d.origin.x - t[0])
                    .abs()
                    .max((d.origin.x + d.size.width - t[0] - t[2]).abs());
                let dy = (d.origin.y + d.size.height - t[1] - t[3]).abs();
                if dx <= 3.0 && dy <= 3.0 {
                    tally.placed += 1;
                } else {
                    tally.misplaced += 1;
                    tally.notes.push(format!(
                        "{}: underline for {from:?} off by {dx:.0}pt across, {dy:.0}pt down",
                        case.name
                    ));
                }
            }
            (None, Some(_)) => {
                tally.unplaced += 1;
                tally
                    .notes
                    .push(format!("{}: no position for {from:?}", case.name));
            }
            // The host cannot say (a textarea): not counted.
            (_, None) => {}
        }
    }

    // One card at a time, first to last, as a user reading down would.
    let mut truth = lab.truth(id)?;
    for (i, ((from, to), &n)) in case.fixes.iter().zip(&nths).enumerate() {
        // Earlier accepts of the same text removed earlier occurrences.
        let n = n - case.fixes[..i]
            .iter()
            .filter(|(f, t)| f == from && !t.contains(from))
            .count();
        let Some(value) = field.value() else {
            tally
                .notes
                .push(format!("{}: the value could not be read", case.name));
            break;
        };
        let Some(fix) = fix_at(&value, from, to, n) else {
            tally.notes.push(format!(
                "{}: {from:?} is gone from the value before its accept",
                case.name
            ));
            break;
        };
        let want = replace_nth(&truth, from, to, n).unwrap_or_default();
        let outcome = accept(lab, id, &want, tally, || {
            write::one(&field, &value, &fix, &paste::Pasteboard)
        })?;
        record(tally, case.name, &format!("{from:?} -> {to:?}"), &outcome);
        if outcome != Result_::Exact {
            break;
        }
        if settled(&field, &want, tally).is_none() {
            tally.notes.push(format!(
                "{}: after {from:?} -> {to:?}, Accessibility still reports the old text",
                case.name
            ));
            break;
        }
        truth = want;
    }

    // "Correct the sentence": each sentence's fixes at once, grouped as the
    // app groups them (`fix::sentence`, which never crosses a line), last
    // sentence first so the others keep their offsets.
    lab.ask(json!({"cmd": "load", "field": id, "text": case.text}))?;
    let field = lab.find(id).with_context(|| format!("{id} not found"))?;
    let mut truth = lab.truth(id)?;
    let Some(value) = settled(&field, &truth, tally) else {
        return Ok(());
    };
    let mut groups: Vec<(Bounds, Vec<Pending>)> = Vec::new();
    for ((from, to), &n) in case.fixes.iter().zip(&nths) {
        let Some(fix) = fix_at(&value, from, to, n) else {
            continue;
        };
        let bounds = gramly_core::sentence(&value, fix.start);
        match groups.iter_mut().find(|(b, _)| *b == bounds) {
            Some((_, group)) => group.push((fix, from, n)),
            None => groups.push((bounds, vec![(fix, from, n)])),
        }
    }
    for (_, group) in groups.into_iter().rev().filter(|(_, g)| g.len() > 1) {
        let Some(value) = settled(&field, &truth, tally) else {
            tally.notes.push(format!(
                "{}: Accessibility still reports the old text",
                case.name
            ));
            break;
        };
        let mut want = truth.clone();
        for (fix, from, n) in group.iter().rev() {
            want = replace_nth(&want, from, &fix.replacement, *n).unwrap_or(want);
        }
        let fixes: Vec<Fix> = group.iter().map(|(f, _, _)| f.clone()).collect();
        let what = format!("the sentence with {}", fixes.len());
        let outcome = accept(lab, id, &want, tally, || {
            write::many(&field, &value, &fixes, &paste::Pasteboard)
        })?;
        record(tally, case.name, &what, &outcome);
        if outcome != Result_::Exact {
            break;
        }
        truth = want;
    }
    Ok(())
}

/// A sentence, as `fix::sentence` bounds it.
type Bounds = (usize, usize);
/// A fix waiting in its sentence: the fix, the text it replaces, and which
/// occurrence of that text it is.
type Pending<'a> = (Fix, &'a str, usize);

/// Runs one accept and waits for the field to settle on `want`, the way the
/// app's audit waits: a paste lands a run-loop turn or two later. Records
/// how long from the click until the field showed it.
fn accept(
    lab: &mut dyn Surface,
    id: &str,
    want: &str,
    tally: &mut Tally,
    write: impl FnOnce() -> Result<String, write::Refusal>,
) -> Result<Result_> {
    if !lab.in_front() {
        bail!(INTERRUPTED);
    }
    lab.check_focus(id)?;
    let before = lab.truth(id)?;
    let started = Instant::now();
    if let Err(why) = write() {
        return Ok(Result_::Refused(why.to_string()));
    }
    loop {
        lab.ask(json!({"cmd": "spin", "seconds": 0.01}))?;
        let got = lab.truth(id)?;
        if got == want {
            tally.millis.push(started.elapsed().as_millis());
            // Let the clipboard's restore run before the next paste.
            lab.ask(json!({"cmd": "spin", "seconds": 0.3}))?;
            return Ok(Result_::Exact);
        }
        if started.elapsed() > Duration::from_millis(1500) {
            if got == before {
                return Ok(Result_::Lost);
            }
            return Ok(Result_::Wrong {
                got,
                want: want.to_string(),
            });
        }
    }
}

fn record(tally: &mut Tally, case: &str, what: &str, outcome: &Result_) {
    match outcome {
        Result_::Exact => {
            tally.exact += 1;
            tally.notes.push(format!("{case}: {what}: exact"));
        }
        Result_::Refused(why) => {
            tally.refused += 1;
            tally.notes.push(format!("{case}: {what}: refused, {why}"));
        }
        Result_::Lost => {
            tally.lost += 1;
            tally
                .notes
                .push(format!("{case}: {what}: lost, the field never changed"));
        }
        Result_::Wrong { got, want } => {
            tally.wrong += 1;
            tally.notes.push(format!(
                "{case}: {what}: WRONG\n      got  {got:?}\n      want {want:?}"
            ));
        }
    }
}

/// How a browser counts and selects text across lines, printed.
fn probe(tab: &mut Browser) -> Result<()> {
    for (id, text) in [
        ("web-plain", "- fix the llimit\n- add a tset\n- ship it"),
        ("web-list", "- fix the llimit\n- add a tset\n- ship it"),
        ("web-chip", "Pick {chip} an oldad for teh campaign"),
    ] {
        tab.ask(json!({"cmd": "load", "field": id, "text": text}))?;
        let field = tab.find(id).context("field not found")?;
        std::thread::sleep(Duration::from_millis(300));
        println!("\n{id}\nvalue {:?}", field.value());
        println!("page  {:?}", tab.truth(id)?);
        println!("{}", field.probe());
        print!("{}", field.tree());
        if let Some(value) = field.value() {
            for word in ["llimit", "tset", "ship", "oldad", "teh"] {
                if let Some(fix) = fix_at(&value, word, "", 0) {
                    let (at, len) = fix.utf16(&value);
                    println!("{word}: {}", field.mapped(at, len));
                }
            }
        }
        for (at, len) in [(10, 6), (16, 0), (17, 3), (18, 4), (20, 4)] {
            field.select_raw(at, len);
            std::thread::sleep(Duration::from_millis(150));
            let dom = tab.ask(json!({"cmd": "sel"}))?;
            println!(
                "select {at}+{len}: AX {:?}  DOM {}",
                field.probe().split("selection ").nth(1),
                dom["text"]
            );
        }
    }
    Ok(())
}

// ------------------------------------------------------------ end to end

/// The texts the real app is given, typed into a browser test page.
const E2E: &[(&str, &str)] = &[
    ("typo", "Can you check teh numbers before the meeting"),
    ("first word", "thi is th teste for me!"),
    ("list", "- fix the llimit\n- add a tset\n- ship it"),
];

/// The installed app, stopped while the lab runs its own copy and started
/// again afterwards, however the run ends: two Gramlys draw two sets of
/// underlines.
struct Paused {
    agent: std::path::PathBuf,
}

impl Paused {
    fn new() -> Self {
        let agent = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join("Library/LaunchAgents/dev.genoux.gramly.plist");
        let _ = Command::new("launchctl")
            .args([
                "bootout",
                &format!("gui/{}/dev.genoux.gramly", unsafe { libc_getuid() }),
            ])
            .status();
        Self { agent }
    }
}

impl Drop for Paused {
    fn drop(&mut self) {
        if self.agent.exists() {
            let _ = Command::new("launchctl")
                .args(["bootstrap", &format!("gui/{}", unsafe { libc_getuid() })])
                .arg(&self.agent)
                .status();
        }
    }
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

fn mouse(kind: objc2_core_graphics::CGEventType, at: CGPoint) {
    use objc2_core_graphics::{CGEvent, CGEventTapLocation, CGMouseButton};
    let event = CGEvent::new_mouse_event(None, kind, at, CGMouseButton::Left);
    CGEvent::post(CGEventTapLocation::HIDEventTap, event.as_deref());
}

fn move_to(at: CGPoint) {
    mouse(objc2_core_graphics::CGEventType::MouseMoved, at);
}

fn click(at: CGPoint) {
    use objc2_core_graphics::CGEventType;
    move_to(at);
    std::thread::sleep(Duration::from_millis(80));
    mouse(CGEventType::LeftMouseDown, at);
    std::thread::sleep(Duration::from_millis(40));
    mouse(CGEventType::LeftMouseUp, at);
}

fn center(r: &[f64]) -> CGPoint {
    CGPoint::new(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
}

fn read_state(path: &std::path::Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The app's state once `ready` holds, within `wait`.
fn await_state(
    path: &std::path::Path,
    wait: Duration,
    ready: impl Fn(&Value) -> bool,
) -> Option<Value> {
    let started = Instant::now();
    loop {
        if let Some(state) = read_state(path)
            && ready(&state)
        {
            return Some(state);
        }
        if started.elapsed() > wait {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn shot(dir: &std::path::Path, name: &str, area: [f64; 4], shots: &mut Vec<String>) {
    let path = dir.join(format!("{name}.png"));
    let region = format!(
        "{},{},{},{}",
        area[0] as i64, area[1] as i64, area[2] as i64, area[3] as i64
    );
    if Command::new("screencapture")
        .args(["-x", "-R", &region])
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success())
    {
        shots.push(path.display().to_string());
    }
}

fn fixes_of(state: &Value) -> Vec<Value> {
    state["fixes"].as_array().cloned().unwrap_or_default()
}

fn fix_of(value: &Value) -> Fix {
    Fix {
        start: value["start"].as_u64().unwrap_or(0) as usize,
        end: value["end"].as_u64().unwrap_or(0) as usize,
        replacement: value["replacement"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        kind: Kind::Correctness,
        message: String::new(),
        source: Source::Ai,
    }
}

/// `gramly lab --e2e`: the real app, in a browser test tab, driven with the
/// mouse like a user, with screenshots.
pub fn e2e(args: &[String]) -> Result<()> {
    if !ax::trusted(false) {
        bail!("this terminal needs Accessibility access to run the lab");
    }
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let wanted = flag("--browser").unwrap_or_else(|| "chrome,safari,zen".into());
    let lab_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/lab");
    let shots_dir = flag("--shots")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| lab_dir.join("shots"));
    std::fs::create_dir_all(&shots_dir)?;
    let state_path = lab_dir.join("state.json");
    let _ = std::fs::remove_file(&state_path);

    let mtm = MainThreadMarker::new().context("not on the main thread")?;
    let primary = NSScreen::screens(mtm)
        .firstObject()
        .map_or(0.0, |s| s.frame().size.height);
    let parked = {
        let at = objc2_app_kit::NSEvent::mouseLocation();
        CGPoint::new(at.x, primary - at.y)
    };

    let _paused = Paused::new();
    let log = std::fs::File::create(lab_dir.join("e2e-app.log"))?;
    let mut app = Command::new(std::env::current_exe()?)
        .env("GRAMLY_STATE", &state_path)
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    std::thread::sleep(Duration::from_secs(2));

    let mut results = Vec::new();
    for (name, app_name, bundle) in BROWSERS {
        if !wanted.split(',').any(|w| w == *name)
            || !std::path::Path::new(&format!("/Applications/{app_name}.app")).exists()
        {
            continue;
        }
        let mut tab = match Browser::open(app_name, bundle) {
            Ok(tab) => tab,
            Err(err) => {
                results.push(json!({"app": app_name, "name": "open", "ok": false,
                    "detail": format!("{err:#}"), "shots": []}));
                continue;
            }
        };
        for (case, text) in E2E {
            let result = e2e_case(&mut tab, &state_path, &shots_dir, app_name, case, text)
                .unwrap_or_else(|err| {
                    json!({"app": app_name, "name": case, "ok": false,
                        "detail": format!("{err:#}"), "shots": []})
                });
            println!(
                "{app_name} · {case}: {} {}",
                if result["ok"] == true { "ok" } else { "FAILED" },
                result["detail"].as_str().unwrap_or_default()
            );
            let stop = result["detail"].as_str() == Some(INTERRUPTED);
            results.push(result);
            if stop {
                break;
            }
        }
        let _ = tab.ask(json!({"cmd": "done"}));
    }

    let _ = app.kill();
    move_to(parked);
    std::fs::write(
        lab_dir.join("e2e.json"),
        serde_json::to_string_pretty(&json!(results))?,
    )?;
    if results.iter().any(|r| r["ok"] != true) {
        bail!("an end-to-end case failed");
    }
    Ok(())
}

fn e2e_case(
    tab: &mut Browser,
    state_path: &std::path::Path,
    shots_dir: &std::path::Path,
    app: &str,
    case: &str,
    text: &str,
) -> Result<Value> {
    let id = "web-plain";
    let slug = format!(
        "{}-{}",
        app.to_lowercase().replace(' ', "-"),
        case.replace(' ', "-")
    );
    let mut shots = Vec::new();
    let mut notes = Vec::new();
    let fail = |detail: String, shots: Vec<String>| {
        Ok(json!({"app": app, "name": case, "ok": false, "detail": detail, "shots": shots}))
    };

    tab.ask(json!({"cmd": "load", "field": id, "text": text}))?;
    if !tab.in_front() {
        return fail(INTERRUPTED.into(), shots);
    }
    let field = tab.find(id).context("the test field was not found")?;
    let frame = field.frame().context("the test field has no frame")?;
    let area = [
        frame.origin.x - 24.0,
        frame.origin.y - 24.0,
        frame.size.width.min(640.0) + 48.0,
        frame.size.height + 260.0,
    ];
    // Out of the way while the underlines come.
    move_to(CGPoint::new(area[0] + 4.0, area[1] + 4.0));
    let loaded = plain(&tab.truth(id)?);
    let started = Instant::now();
    let Some(state) = await_state(state_path, Duration::from_secs(12), |s| {
        plain(s["text"].as_str().unwrap_or_default()) == loaded
            && s["ai_current"] == true
            && fixes_of(s).iter().any(|f| f["drawn"].is_array())
    }) else {
        shot(shots_dir, &format!("{slug}-nothing"), area, &mut shots);
        return fail("no underline within 12s".into(), shots);
    };
    let waited = started.elapsed().as_millis();
    let value = state["text"].as_str().unwrap_or_default().to_string();

    // Each underline against where the page draws its word.
    for f in fixes_of(&state) {
        let (Some(drawn), Some(original)) = (f["drawn"].as_array(), f["original"].as_str()) else {
            continue;
        };
        let start = f["start"].as_u64().unwrap_or(0) as usize;
        let before: String = value.chars().take(start).collect();
        let nth = before.matches(original).count();
        let truth = tab.ask(json!({"cmd": "rect", "field": id, "word": original, "nth": nth}))?;
        if let (Some(t), Some(d)) = (truth["rect"].as_array(), drawn.get(0..4)) {
            let t: Vec<f64> = t.iter().filter_map(Value::as_f64).collect();
            let d: Vec<f64> = d.iter().filter_map(Value::as_f64).collect();
            let dx = (d[0] - t[0]).abs().max((d[0] + d[2] - t[0] - t[2]).abs());
            let dy = (d[1] + d[3] - t[1] - t[3]).abs();
            if dx > 3.0 || dy > 3.0 {
                notes.push(format!(
                    "underline for {original:?} off by {dx:.0}pt, {dy:.0}pt"
                ));
            }
        }
    }
    shot(shots_dir, &format!("{slug}-1-underlines"), area, &mut shots);

    // Hover the first underline, open its card, accept.
    let Some(target) = fixes_of(&state).into_iter().find(|f| f["drawn"].is_array()) else {
        return fail("no drawn fix".into(), shots);
    };
    let drawn: Vec<f64> = target["drawn"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    move_to(center(&drawn));
    let Some(card) = await_state(state_path, Duration::from_secs(3), |s| {
        s["rows"].as_array().is_some_and(|r| !r.is_empty())
    }) else {
        shot(shots_dir, &format!("{slug}-no-card"), area, &mut shots);
        return fail("hovering the underline opened no card".into(), shots);
    };
    shot(shots_dir, &format!("{slug}-2-card"), area, &mut shots);
    let rows = card["rows"].as_array().cloned().unwrap_or_default();
    let Some(accept) = rows.iter().find(|r| {
        r["action"]
            .as_str()
            .is_some_and(|a| a.starts_with("accept"))
    }) else {
        return fail("the card has no accept button".into(), shots);
    };
    let sentence = accept["action"] == "acceptSentence:";
    let chosen = fix_of(&target);
    let accepted: Vec<Fix> = if sentence {
        let bounds = gramly_core::sentence(&value, chosen.start);
        fixes_of(&card)
            .iter()
            .map(fix_of)
            .filter(|f| f.start >= bounds.0 && f.end <= bounds.1.max(bounds.0))
            .collect()
    } else {
        vec![chosen]
    };
    let want = gramly_core::apply(&value, &accepted);
    let before = plain(&tab.truth(id)?);
    let rect: Vec<f64> = accept["rect"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    click(center(&rect));
    let started_accept = Instant::now();
    let landed = loop {
        let got = plain(&tab.truth(id)?);
        if got == plain(&want) {
            break Ok(started_accept.elapsed().as_millis());
        }
        if started_accept.elapsed() > Duration::from_secs(3) {
            break Err(got);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    move_to(CGPoint::new(area[0] + 4.0, area[1] + 4.0));
    let shown = match landed {
        Ok(ms) => ms,
        Err(got) => {
            shot(shots_dir, &format!("{slug}-3-after"), area, &mut shots);
            let what = if got == before {
                "the click did nothing".to_string()
            } else {
                format!("the page holds {got:?}, not {:?}", plain(&want))
            };
            return fail(what, shots);
        }
    };

    // The model reads the corrected text again; nothing it offers may sit
    // on what was just accepted - a fix on top of a fix.
    let after = await_state(state_path, Duration::from_secs(6), |s| {
        plain(s["text"].as_str().unwrap_or_default()) == plain(&want) && s["ai_current"] == true
    });
    shot(shots_dir, &format!("{slug}-3-after"), area, &mut shots);
    if let Some(after) = after {
        let mut shift: isize = 0;
        for fix in &accepted {
            let start = (fix.start as isize + shift) as usize;
            let end = start + fix.replacement.chars().count();
            shift += fix.replacement.chars().count() as isize - (fix.end - fix.start) as isize;
            for f in fixes_of(&after) {
                let (s, e) = (
                    f["start"].as_u64().unwrap_or(0) as usize,
                    f["end"].as_u64().unwrap_or(0) as usize,
                );
                if s < end && start < e {
                    notes.push(format!(
                        "a fix on top of the accepted {:?}: {:?} -> {:?}",
                        fix.replacement, f["original"], f["replacement"]
                    ));
                }
            }
        }
    } else {
        notes.push("the model did not answer again within 6s".into());
    }

    let ok = notes.is_empty();
    let detail = format!(
        "underlines {waited}ms after the text, the accept showed in {shown}ms{}",
        if ok {
            String::new()
        } else {
            format!("; {}", notes.join("; "))
        }
    );
    Ok(json!({"app": app, "name": case, "ok": ok, "detail": detail, "shots": shots}))
}
