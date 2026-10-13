//! Which language a paragraph is in, so each pass only sees what it can judge.
//!
//! Harper is English-only: given French it flags every word as a misspelling.
//! The model handles both, but Flow measured that naming the detected language
//! in the prompt holds it to that language far better than a general "keep
//! the language" rule - a code-switched sentence is what tips it into
//! translating. Same detector as Flow (whatlang), same "reliable only" rule.

use whatlang::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    French,
    Other(Lang),
    /// Too short or too mixed to tell. Left to the model.
    Unknown,
}

/// English or French, the two this was built for. Detecting among every
/// language whatlang knows called a plain ten-word English sentence
/// unreliable; between two it is decisive from a few words.
const LANGUAGES: [Lang; 2] = [Lang::Eng, Lang::Fra];

/// Below this, a two-way guess is a coin toss and is reported as such.
const CONFIDENT: f64 = 0.5;

pub fn detect(text: &str) -> Language {
    let detector = whatlang::Detector::with_allowlist(LANGUAGES.to_vec());
    match detector.detect(text) {
        Some(info) if info.is_reliable() || info.confidence() >= CONFIDENT => match info.lang() {
            Lang::Eng => Language::English,
            Lang::Fra => Language::French,
            other => Language::Other(other),
        },
        _ => Language::Unknown,
    }
}

impl Language {
    /// Whether Harper's English rules apply: only to text that is clearly
    /// English. "ERT CEst cool mais jpense pas." is too short to call, and
    /// treating unknown as English is how Harper got to it; unclear text now
    /// waits the second for the model instead.
    pub fn local_pass(self) -> bool {
        self == Language::English
    }

    /// The name the prompt uses, when there is one worth stating.
    pub fn name(self) -> Option<&'static str> {
        match self {
            Language::English => Some("English"),
            Language::French => Some("French"),
            Language::Other(lang) => Some(lang.eng_name()),
            Language::Unknown => None,
        }
    }
}

/// The distinct languages of a text's sentences, in order of appearance,
/// ignoring sentences too short to tell.
pub fn sentence_languages(text: &str) -> Vec<Language> {
    let mut found = Vec::new();
    let len = text.chars().count();
    let mut at = 0;
    while at < len {
        let (start, end) = crate::fix::sentence(text, at);
        let sentence: String = text.chars().skip(start).take(end - start).collect();
        let language = detect(&sentence);
        if language != Language::Unknown && !found.contains(&language) {
            found.push(language);
        }
        at = end.max(at + 1);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_french_from_english() {
        assert_eq!(
            detect("Je pense que nous devrions partir demain matin avant le petit déjeuner."),
            Language::French
        );
        assert_eq!(
            detect("I think we should leave tomorrow morning before breakfast."),
            Language::English
        );
        assert_eq!(detect("their is to many emails"), Language::English);
        assert_eq!(detect("je suis aller au marché"), Language::French);
    }
}
