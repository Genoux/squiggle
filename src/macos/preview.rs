//! `gramly preview [dir]`: every card and the dictionary drawn to PNG, in
//! the light and dark appearances, without a field or a screen to take over.
//! For changing how they look: the files are what the app draws, from the
//! same code, in an off-screen window.

use crate::macos::dictionary::Dictionary;
use crate::macos::overlay::{self, SHADE};
use crate::macos::theme;
use anyhow::{Context, Result, anyhow};
use gramly_core::{Fix, Kind, Source};
use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication,
    NSApplicationActivationPolicy, NSBackingStoreType, NSBitmapImageFileType, NSBitmapImageRep,
    NSColor, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImageAlphaInfo,
};
use objc2_foundation::{NSDictionary, NSObject};
use objc2_quartz_core::CATransaction;
use std::path::{Path, PathBuf};

/// Pixels per point in the files.
const SCALE: f64 = 2.0;
/// Room around what is drawn.
const MARGIN: f64 = 24.0;

fn fix(text: &str, gone: &str, replacement: &str, kind: Kind, message: &str) -> Fix {
    let byte = text.find(gone).expect("sample text holds the word");
    let start = text[..byte].chars().count();
    Fix {
        start,
        end: start + gone.chars().count(),
        replacement: replacement.into(),
        kind,
        message: message.into(),
        source: Source::Ai,
    }
}

/// (file name, text, the fixes in the hovered fix's sentence, which is hovered)
fn samples() -> Vec<(&'static str, &'static str, Vec<Fix>, usize)> {
    let mail = "Did you recieve the photos? Can you send me the the link for the frame.io draft when you have a sec?";
    let removal = {
        let byte = mail.find("the the").expect("sample") + 4;
        let start = mail[..byte].chars().count();
        Fix {
            start,
            end: start + 4,
            replacement: String::new(),
            kind: Kind::Correctness,
            message: String::new(),
            source: Source::Ai,
        }
    };
    let trip = "i went to paris on monday and it was great.";
    let improve =
        "I think it would maybe be better if we could try to move the meeting to Thursday instead.";
    let spacing = "So yes  was the answer.";
    let question = "Can you think about conservation for now?Due to the fact that we ship soon.";
    vec![
        (
            "spelling",
            mail,
            vec![fix(mail, "recieve", "receive", Kind::Spelling, "")],
            0,
        ),
        ("removal", mail, vec![removal], 0),
        (
            "sentence",
            trip,
            vec![
                fix(trip, "i went", "I went", Kind::Correctness, ""),
                fix(trip, "paris", "Paris", Kind::Spelling, ""),
                fix(trip, "monday", "Monday", Kind::Spelling, ""),
            ],
            1,
        ),
        (
            "improve",
            improve,
            vec![fix(
                improve,
                "it would maybe be better if we could try to",
                "we should",
                Kind::Clarity,
                "",
            )],
            0,
        ),
        (
            "spacing",
            spacing,
            vec![fix(spacing, "  ", " ", Kind::Correctness, "")],
            0,
        ),
        (
            "punctuation",
            question,
            vec![fix(
                question,
                "now?Due",
                "now? Due",
                Kind::Correctness,
                "Correct your punctuation",
            )],
            0,
        ),
    ]
}

pub fn run(args: &[String]) -> Result<()> {
    let mtm = MainThreadMarker::new().ok_or_else(|| anyhow!("not on the main thread"))?;
    let dir = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/preview"));
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let target = NSObject::new();

    for (mode, name) in [
        ("light", unsafe { NSAppearanceNameAqua }),
        ("dark", unsafe { NSAppearanceNameDarkAqua }),
    ] {
        app.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
        let p = theme::palette(mtm);
        let ground = if mode == "dark" {
            theme::Rgba {
                r: 0.12,
                g: 0.12,
                b: 0.13,
                a: 1.0,
            }
        } else {
            theme::Rgba {
                r: 0.91,
                g: 0.91,
                b: 0.93,
                a: 1.0,
            }
        };

        for (file, text, fixes, hovered) in samples() {
            let group: Vec<(usize, Fix)> = fixes.into_iter().enumerate().collect();
            let card = overlay::card(hovered, &group, text, &target, p, mtm)
                .ok_or_else(|| anyhow!("no card for {file}"))?;
            let size = card.view.frame().size;
            let shade = overlay::shade(size, p, mtm);
            let host = backdrop(shade.frame().size, ground, mtm);
            shade.setFrameOrigin(CGPoint::new(MARGIN, MARGIN));
            host.addSubview(&shade);
            card.view
                .setFrameOrigin(CGPoint::new(MARGIN + SHADE.0, MARGIN + SHADE.3));
            host.addSubview(&card.view);
            save(&host, &dir.join(format!("{file}-{mode}.png")), mtm)?;
        }

        for (file, words) in [
            (
                "dictionary",
                &[
                    "Fieldtrip",
                    "Gramly",
                    "Hyprland",
                    "inBeat",
                    "tmrw",
                    "Wayland",
                    "Zen",
                ][..],
            ),
            ("dictionary-empty", &[][..]),
        ] {
            let mut dictionary = Dictionary::new(&target, mtm);
            dictionary.preview(words, &target, mtm);
            let content = dictionary
                .content()
                .ok_or_else(|| anyhow!("the dictionary has no content"))?;
            let host = backdrop(content.frame().size, ground, mtm);
            content.removeFromSuperview();
            content.setFrameOrigin(CGPoint::new(MARGIN, MARGIN));
            host.addSubview(&content);
            save(&host, &dir.join(format!("{file}-{mode}.png")), mtm)?;
        }
    }
    println!("{}", dir.display());
    Ok(())
}

/// A view `size` plus the margin, filled with `ground`.
fn backdrop(size: CGSize, ground: theme::Rgba, mtm: MainThreadMarker) -> Retained<NSView> {
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(size.width + MARGIN * 2.0, size.height + MARGIN * 2.0),
        ),
    );
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.setBackgroundColor(Some(&theme::cg(ground)));
    }
    view
}

/// Draws `view` in an off-screen window and writes it to `path`.
fn save(view: &NSView, path: &Path, mtm: MainThreadMarker) -> Result<()> {
    let size = view.frame().size;
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            CGRect::new(CGPoint::new(-30000.0, -30000.0), size),
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setContentView(Some(view));
    window.orderFrontRegardless();
    window.displayIfNeeded();
    CATransaction::flush();

    let (w, h) = (
        (size.width * SCALE).round() as usize,
        (size.height * SCALE).round() as usize,
    );
    let space = CGColorSpace::new_device_rgb().ok_or_else(|| anyhow!("no colour space"))?;
    let context = unsafe {
        CGBitmapContextCreate(
            std::ptr::null_mut(),
            w,
            h,
            8,
            0,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }
    .ok_or_else(|| anyhow!("no bitmap context"))?;
    CGContext::scale_ctm(Some(&context), SCALE, SCALE);
    let layer = view
        .layer()
        .ok_or_else(|| anyhow!("the view has no layer"))?;
    layer.renderInContext(&context);
    let image = CGBitmapContextCreateImage(Some(&context)).ok_or_else(|| anyhow!("no image"))?;
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .ok_or_else(|| anyhow!("no PNG"))?;
    std::fs::write(path, png.to_vec()).with_context(|| format!("writing {}", path.display()))?;
    window.orderOut(None);
    Ok(())
}
