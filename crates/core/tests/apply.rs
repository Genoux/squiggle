//! Accepting fixes, the way a user does: one card at a time in any order,
//! or a sentence at once. Whatever the order, the text must end exactly as
//! corrected, nothing outside a fix may move, and every underline still
//! standing must still sit on the words it was about.
//!
//! The texts are the shapes that broke in use: bullets, numbered lists,
//! emoji before a typo, French, doubled spaces, fixes side by side.

use squiggle_core::{Fix, apply, diff, rebase};

/// (what was typed, what the model returned)
const CASES: &[(&str, &str)] = &[
    ("Can you check teh numbers", "Can you check the numbers"),
    (
        "i went to paris on monday it was great\n",
        "I went to Paris on Monday. It was great.\n",
    ),
    (
        "Pick an oldad from frame.io",
        "Pick an old ad from frame.io",
    ),
    (
        "Create a draft. Pickan old ad",
        "Create a draft. Pick an old ad",
    ),
    (
        "Could you send me the the link",
        "Could you send me the link",
    ),
    ("Hey yes  was about to say", "Hey, yes, I was about to say"),
    ("Let me no when your free", "Let me know when you're free"),
    // Lists: the marker and the indent stay exactly where they are.
    (
        "- fix the llimit\n- add a tset\n- ship it",
        "- fix the limit\n- add a test\n- ship it",
    ),
    (
        "• first iteam\n• second iteam\n",
        "• first item\n• second item\n",
    ),
    (
        "\t• nested iteam\n\t• another one",
        "\t• nested item\n\t• another one",
    ),
    (
        "1. recieve the file\n2. sned it back",
        "1. receive the file\n2. send it back",
    ),
    (
        "Todo:\n- llimit the scope\n  - and teh rest",
        "Todo:\n- limit the scope\n  - and the rest",
    ),
    // Emoji and other wide characters before and between fixes.
    (
        "🚀 Shiping teh release 🎉 tommorow",
        "🚀 Shipping the release 🎉 tomorrow",
    ),
    ("👍🏽 sounds god, see u", "👍🏽 sounds good, see u"),
    // French, accents and a no-break space before "?".
    (
        "Je suis aller au marché et j'ai acheter du pain",
        "Je suis allé au marché et j'ai acheté du pain",
    ),
    (
        "tu viens ce soir ou pas\n",
        "Tu viens ce soir ou pas\u{a0}?\n",
    ),
    ("Est ce que tu peut venir", "Est-ce que tu peux venir"),
    // Spacing.
    ("Two  spaces and  more", "Two spaces and more"),
    ("Trailing space here ", "Trailing space here "),
    (
        "No space after comma,like this",
        "No space after comma, like this",
    ),
    // Fixes that touch each other.
    ("its a i think", "it's a I think"),
    (
        "Their is a problem with there account",
        "There is a problem with their account",
    ),
    (
        "the results was better then expected",
        "The results were better than expected",
    ),
    // Several paragraphs.
    (
        "Hi team,\n\nthe deploy is tonight, if anything break we roll back\n\nThanks",
        "Hi team,\n\nThe deploy is tonight. If anything breaks, we roll back\n\nThanks",
    ),
];

fn word(text: &str, fix: &Fix) -> String {
    fix.original(text)
}

/// Accepts the fixes in `order` (indices into the original list), rebasing
/// the rest after each accept the way the app does when the text changes.
/// Returns the final text, or what went wrong.
fn accept_in(text: &str, fixes: &[Fix], order: &[usize]) -> Result<String, String> {
    let mut current = text.to_string();
    // Each fix with its index, as the app keeps them.
    let mut standing: Vec<(usize, Fix)> = fixes.iter().cloned().enumerate().collect();
    for &next in order {
        let Some(position) = standing.iter().position(|(i, _)| *i == next) else {
            return Err(format!("fix {next} vanished before it could be accepted"));
        };
        let (_, fix) = standing.remove(position);
        let before: Vec<(usize, String)> = standing
            .iter()
            .map(|(i, f)| (*i, word(&current, f)))
            .collect();
        let updated = apply(&current, std::slice::from_ref(&fix));
        let rest: Vec<Fix> = standing.iter().map(|(_, f)| f.clone()).collect();
        let moved = rebase(&current, &updated, &rest);
        if moved.len() != rest.len() {
            return Err(format!(
                "accepting {:?} -> {:?} took {} other underline(s) with it",
                word(&current, &fix),
                fix.replacement,
                rest.len() - moved.len()
            ));
        }
        for ((i, was), now) in before.iter().zip(&moved) {
            let is = word(&updated, now);
            if *was != is {
                return Err(format!(
                    "after accepting {:?}, fix {i}'s underline moved from {was:?} to {is:?}",
                    fix.replacement
                ));
            }
        }
        standing = standing.iter().map(|(i, _)| *i).zip(moved).collect();
        current = updated;
    }
    Ok(current)
}

fn orders(n: usize) -> Vec<Vec<usize>> {
    if n <= 4 {
        // Every order.
        let mut all = vec![vec![]];
        for _ in 0..n {
            all = all
                .into_iter()
                .flat_map(|o: Vec<usize>| {
                    (0..n)
                        .filter(|i| !o.contains(i))
                        .map(|i| {
                            let mut next = o.clone();
                            next.push(i);
                            next
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
        }
        all
    } else {
        // First to last, last to first, and outside in.
        let forward: Vec<usize> = (0..n).collect();
        let backward: Vec<usize> = (0..n).rev().collect();
        let mut zigzag = Vec::new();
        let (mut lo, mut hi) = (0, n - 1);
        while lo <= hi {
            zigzag.push(lo);
            if lo != hi {
                zigzag.push(hi);
            }
            lo += 1;
            hi = hi.saturating_sub(1);
            if hi < lo {
                break;
            }
        }
        vec![forward, backward, zigzag]
    }
}

#[test]
fn every_order_of_accepts_ends_exactly_as_corrected() {
    let mut failures = Vec::new();
    for (text, corrected) in CASES {
        let fixes = diff::fixes(text, corrected);
        assert!(
            !fixes.is_empty() || text == corrected,
            "no fixes for {text:?}"
        );
        if apply(text, &fixes) != *corrected {
            failures.push(format!(
                "{text:?}: the fixes found do not make {corrected:?}"
            ));
            continue;
        }
        for order in orders(fixes.len()) {
            match accept_in(text, &fixes, &order) {
                Ok(done) if done == *corrected => {}
                Ok(done) => failures.push(format!(
                    "{text:?} in order {order:?}: ended as {done:?}, not {corrected:?}"
                )),
                Err(why) => failures.push(format!("{text:?} in order {order:?}: {why}")),
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn a_fix_never_reaches_across_a_line() {
    for (text, corrected) in CASES {
        for fix in diff::fixes(text, corrected) {
            let original = fix.original(text);
            assert!(
                !original.contains('\n') && !fix.replacement.contains('\n'),
                "{text:?}: {original:?} -> {:?} crosses a line",
                fix.replacement
            );
        }
    }
}

#[test]
fn list_markers_and_indents_never_change() {
    let marker = |line: &str| -> String {
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];
        let bullet: String = trimmed
            .chars()
            .take_while(|c| matches!(c, '-' | '•' | '*' | '.' | ')') || c.is_ascii_digit())
            .collect();
        format!("{indent}{bullet}")
    };
    for (text, corrected) in CASES {
        let fixes = diff::fixes(text, corrected);
        let done = apply(text, &fixes);
        for (a, b) in text.lines().zip(done.lines()) {
            assert_eq!(marker(a), marker(b), "{text:?}: a line's marker moved");
        }
    }
}

#[test]
fn utf16_ranges_select_the_same_text() {
    for (text, corrected) in CASES {
        let units: Vec<u16> = text.encode_utf16().collect();
        for fix in diff::fixes(text, corrected) {
            let (at, len) = fix.utf16(text);
            let selected = String::from_utf16(&units[at..at + len]).unwrap();
            assert_eq!(selected, fix.original(text), "{text:?}");
        }
    }
}
