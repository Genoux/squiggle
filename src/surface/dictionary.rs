//! What the dictionary window says, the same on every platform.

pub const TITLE: &str = "Dictionary";
pub const SUBTITLE: &str = "Gramly never flags these words.";
pub const ADD: &str = "Add";
pub const PLACEHOLDER: &str = "Add a word";
pub const EMPTY: &str = "No words yet. Words you add here, or from a card, show up in this list.";

/// "3 words"; nothing for an empty list, which says so itself.
pub fn count(words: usize) -> String {
    match words {
        0 => String::new(),
        1 => "1 word".into(),
        n => format!("{n} words"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_is_singular_for_one_and_empty_for_none() {
        assert_eq!(count(0), "");
        assert_eq!(count(1), "1 word");
        assert_eq!(count(12), "12 words");
    }
}
