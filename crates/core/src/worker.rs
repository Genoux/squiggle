//! Checking off the UI thread, in two speeds.
//!
//! The local pass answers in milliseconds and the model in about a second, so
//! they are two threads: underlines from Harper never wait behind a network
//! call, and the model's answer arrives as a second, merged outcome for the
//! same generation. Both threads coalesce to the newest request - text that
//! was typed over while it waited is not worth an answer.
//!
//! The model is asked per paragraph and answers are cached by the paragraph's
//! exact text. Typing in one paragraph of a long email then costs one request
//! for that paragraph, and moving focus away and back costs nothing.

use crate::config::{self, Config};
use crate::fix::Fix;
use crate::{Local, ai};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

#[derive(Debug, Clone)]
pub struct Request {
    /// Bumped by the caller for every new text; outcomes carry it back so a
    /// late answer for old text is recognisable and dropped.
    pub generation: u64,
    pub text: String,
}

#[derive(Debug)]
pub enum Outcome {
    Local {
        generation: u64,
        fixes: Vec<Fix>,
    },
    Ai {
        generation: u64,
        fixes: Vec<Fix>,
    },
    /// The model pass failed; the local fixes stand. Carries the reason for
    /// the menu, never the key.
    AiFailed {
        generation: u64,
        reason: String,
    },
}

pub struct Worker {
    requests: Sender<Request>,
    outcomes: Receiver<Outcome>,
}

/// Paragraph caches are cleared past this many entries rather than evicted
/// one by one: it is a cost cap, not a hit-rate optimisation.
const CACHE_LIMIT: usize = 512;
/// Uncached paragraphs asked at once on a first look at a long document.
const PARALLEL: usize = 4;

impl Worker {
    pub fn spawn() -> Self {
        let (requests, local_in) = channel::<Request>();
        let (ai_out, ai_in) = channel::<Request>();
        let (outcome_tx, outcomes) = channel::<Outcome>();

        let local_tx = outcome_tx.clone();
        std::thread::Builder::new()
            .name("gramly-local".into())
            .spawn(move || local_loop(local_in, ai_out, local_tx))
            .expect("spawning the local checker");
        std::thread::Builder::new()
            .name("gramly-ai".into())
            .spawn(move || ai_loop(ai_in, outcome_tx))
            .expect("spawning the model checker");

        Self { requests, outcomes }
    }

    pub fn submit(&self, generation: u64, text: String) {
        let _ = self.requests.send(Request { generation, text });
    }

    pub fn try_recv(&self) -> Option<Outcome> {
        self.outcomes.try_recv().ok()
    }
}

fn newest(first: Request, rx: &Receiver<Request>) -> Request {
    let mut latest = first;
    while let Ok(next) = rx.try_recv() {
        latest = next;
    }
    latest
}

fn local_loop(rx: Receiver<Request>, ai: Sender<Request>, out: Sender<Outcome>) {
    let mut dialect = None;
    let mut local: Option<Local> = None;
    while let Ok(first) = rx.recv() {
        let request = newest(first, &rx);
        // Read per request, like Flow reads its style per dictation: a word
        // added or a switch flipped applies on the next pause, not the next
        // launch. The files are a few hundred bytes.
        let config = Config::load().unwrap_or_default();
        if !config.enabled {
            let _ = out.send(Outcome::Local {
                generation: request.generation,
                fixes: Vec::new(),
            });
            continue;
        }
        let wanted = config.harper_dialect();
        if dialect != Some(wanted) {
            local = Some(Local::new(wanted));
            dialect = Some(wanted);
        }
        let checker = local.as_mut().expect("built above");
        checker.set_known_words(config::dictionary());
        let fixes = checker.check(&request.text);
        let _ = out.send(Outcome::Local {
            generation: request.generation,
            fixes,
        });
        if config.ai && config.openrouter_key.is_some() {
            let _ = ai.send(request);
        }
    }
}

fn ai_loop(rx: Receiver<Request>, out: Sender<Outcome>) {
    let mut cache: HashMap<String, Vec<Fix>> = HashMap::new();
    let mut pending: Option<Request> = None;
    loop {
        let first = match pending.take() {
            Some(request) => request,
            None => match rx.recv() {
                Ok(request) => request,
                Err(_) => return,
            },
        };
        let request = newest(first, &rx);
        let config = Config::load().unwrap_or_default();
        let Some(key) = config.openrouter_key.filter(|_| config.ai) else {
            continue;
        };

        let pieces = paragraphs(&request.text);
        let missing: Vec<&str> = pieces
            .iter()
            .map(|(_, p)| *p)
            .filter(|p| !cache.contains_key(*p))
            .collect();

        let mut failure = None;
        for batch in missing.chunks(PARALLEL) {
            let answers: Vec<(String, anyhow::Result<Vec<Fix>>)> = std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .iter()
                    .map(|p| {
                        let key = &key;
                        scope.spawn(move || (p.to_string(), ai::check(key, p)))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().expect("checker"))
                    .collect()
            });
            for (paragraph, answer) in answers {
                match answer {
                    Ok(fixes) => {
                        if cache.len() >= CACHE_LIMIT {
                            cache.clear();
                        }
                        cache.insert(paragraph, fixes);
                    }
                    Err(err) => failure = Some(err.to_string()),
                }
            }
            // Newer text arrived while this batch was out: answer that instead.
            // What was fetched stays cached, so the work is not lost.
            match rx.try_recv() {
                Ok(next) => {
                    pending = Some(next);
                    break;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if pending.is_some() {
            continue;
        }
        if let Some(reason) = failure {
            let _ = out.send(Outcome::AiFailed {
                generation: request.generation,
                reason,
            });
        }

        let mut fixes = Vec::new();
        for (offset, paragraph) in &pieces {
            if let Some(found) = cache.get(*paragraph) {
                fixes.extend(found.iter().cloned().map(|mut fix| {
                    fix.start += offset;
                    fix.end += offset;
                    fix
                }));
            }
        }
        let _ = out.send(Outcome::Ai {
            generation: request.generation,
            fixes,
        });
    }
}

/// Non-blank lines with the char offset each starts at.
pub fn paragraphs(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split('\n') {
        if !line.trim().is_empty() {
            out.push((offset, line));
        }
        offset += line.chars().count() + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraph_offsets_are_chars() {
        let text = "é one\n\ntwo";
        let parts = paragraphs(text);
        assert_eq!(parts, vec![(0, "é one"), (7, "two")]);
        let second: String = text.chars().skip(7).collect();
        assert_eq!(second, "two");
    }
}
