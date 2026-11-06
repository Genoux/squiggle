//! OpenRouter, the same way Flow reaches it.
//!
//! curl rather than an HTTP crate: one round trip with a JSON reply, and an
//! async runtime would buy nothing back. The key never reaches curl's argv -
//! the process list is readable by other local users - so options go in on
//! stdin via `-K -` and the body in a 0600 temporary file passed by path.

use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const API: &str = "https://openrouter.ai/api/v1";

/// Which model answers, on which host, thinking how hard. Pinned to the
/// host it was measured against: a silent reroute to another host serving
/// the same id is a different machine, and the prompt was tuned against
/// this one. The app uses [`Route::default`]; the benchmark
/// (`examples/bench.rs`) tries others.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Route {
    pub model: String,
    /// An OpenRouter provider slug ("anthropic", "google-ai-studio").
    pub provider: String,
    /// "low", "medium" or "high"; "none" turns reasoning off; absent sends
    /// nothing and leaves the model's default.
    #[serde(default)]
    pub reasoning: Option<String>,
}

impl Default for Route {
    fn default() -> Self {
        // Chosen by `examples/bench.rs` (2026-10-13, 60 cases, 3 runs, three
        // passes): as precise as Haiku 5.5 or more, the same F0.5, and a
        // slowest 5% of 1.2-1.5s against Haiku's 3.2-3.7s.
        Self {
            model: "google/gemini-3.5-flash-lite".into(),
            provider: "google-ai-studio".into(),
            reasoning: Some("low".into()),
        }
    }
}

/// A reply and what it cost, in dollars, when OpenRouter says.
pub struct Answer {
    pub value: serde_json::Value,
    pub cost: Option<f64>,
}

/// One structured request: `schema` is a JSON Schema the reply must satisfy.
pub fn structured(
    key: &str,
    system: &str,
    user: &str,
    schema: &serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value> {
    structured_on(&Route::default(), key, system, user, schema, timeout).map(|a| a.value)
}

/// [`structured`], on any route.
pub fn structured_on(
    route: &Route,
    key: &str,
    system: &str,
    user: &str,
    schema: &serde_json::Value,
    timeout: Duration,
) -> Result<Answer> {
    let mut body = serde_json::json!({
        "model": route.model,
        "max_tokens": 2048,
        "provider": {"only": [route.provider], "allow_fallbacks": false},
        "usage": {"include": true},
        "response_format": {
            "type": "json_schema",
            "json_schema": {"name": "edits", "strict": true, "schema": schema},
        },
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    });
    match route.reasoning.as_deref() {
        None => {}
        Some("none") => body["reasoning"] = serde_json::json!({"enabled": false}),
        Some(effort) => body["reasoning"] = serde_json::json!({"effort": effort}),
    }
    let reply = post(key, "/chat/completions", &body.to_string(), timeout)?;
    let content = parse_chat(&reply)?;
    let cost = reply.pointer("/usage/cost").and_then(|c| c.as_f64());
    let value = serde_json::from_str(&content)
        .context("the model's reply is not the JSON it was asked for")?;
    Ok(Answer { value, cost })
}

/// The reply's text, or why there isn't one. A truncated reply is a failure:
/// half a list of edits is a list whose last entry may be cut mid-word.
fn parse_chat(reply: &serde_json::Value) -> Result<String> {
    if let Some(error) = reply.get("error") {
        let message = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("no reason given");
        match error.get("code").and_then(|c| c.as_u64()) {
            Some(code) => bail!("OpenRouter refused the text ({code}): {message}"),
            None => bail!("OpenRouter refused the text: {message}"),
        }
    }
    let Some(choice) = reply.get("choices").and_then(|c| c.get(0)) else {
        bail!("OpenRouter sent no choices");
    };
    match choice.get("finish_reason").and_then(|r| r.as_str()) {
        Some("stop") => {}
        Some(reason) => bail!("the model stopped early ({reason})"),
        None => bail!("the model did not say why it stopped"),
    }
    let text = choice
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or_default();
    if text.trim().is_empty() {
        bail!("the model returned nothing");
    }
    Ok(text.trim().to_string())
}

/// A failure only the writer can end, as the menu offers to: money on the
/// account, or a key that works. Anything else - the network, a busy
/// provider - passes on its own, and the menu says nothing about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    Credits,
    Key,
}

/// What `reason`, a model check's failure, asks of the writer, if anything.
/// OpenRouter answers 402 when the credits or the key's limit run out
/// ("This request requires more credits, or fewer max_tokens") and 401
/// for a key it does not know.
pub fn blocked(reason: &str) -> Option<Blocked> {
    if !reason.starts_with("OpenRouter refused the text") {
        return None;
    }
    if reason.contains("(402)") || reason.to_lowercase().contains("credits") {
        Some(Blocked::Credits)
    } else if reason.contains("(401)") {
        Some(Blocked::Key)
    } else {
        None
    }
}

fn post(key: &str, path: &str, body: &str, timeout: Duration) -> Result<serde_json::Value> {
    let scratch = scratch_file();
    write_private(&scratch, body.as_bytes()).context("staging the request")?;
    let result = send(key, path, &scratch, timeout);
    let _ = std::fs::remove_file(&scratch);
    result
}

pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

fn send(key: &str, path: &str, scratch: &Path, timeout: Duration) -> Result<serde_json::Value> {
    if !valid_key(key) {
        bail!("Invalid OpenRouter key");
    }
    let mut child = Command::new("curl")
        .args(["--silent", "--show-error", "-K", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("curl is not installed")?;

    let options = format!(
        "url = \"{API}{path}\"\n\
         request = \"POST\"\n\
         header = \"Content-Type: application/json\"\n\
         header = \"Authorization: Bearer {key}\"\n\
         header = \"X-Title: Squiggle\"\n\
         data-binary = \"@{}\"\n\
         max-time = {}\n\
         fail-with-body\n",
        scratch
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r"),
        timeout.as_secs_f64().max(0.001),
    );
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(options.as_bytes())
        .context("handing curl its options")?;

    let output = child.wait_with_output().context("running curl")?;
    if !output.status.success() && output.stdout.is_empty() {
        let reason = String::from_utf8_lossy(&output.stderr);
        bail!("could not reach OpenRouter: {}", reason.trim());
    }
    serde_json::from_slice(&output.stdout).context("OpenRouter sent something that is not JSON")
}

fn scratch_file() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "squiggle-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_with_a_quote_never_reaches_curl() {
        assert!(!valid_key("sk-or-\"; url = evil"));
        assert!(valid_key("sk-or-v1-abc123_DEF"));
    }

    #[test]
    fn only_a_refusal_the_writer_can_end_is_blocked() {
        let credits = serde_json::json!({"error": {"code": 402, "message":
            "This request requires more credits, or fewer max_tokens. You requested up to 2048 tokens, but can only afford 989."}});
        let reason = parse_chat(&credits).unwrap_err().to_string();
        assert_eq!(blocked(&reason), Some(Blocked::Credits), "{reason}");
        let key = serde_json::json!({"error": {"code": 401, "message": "User not found."}});
        let reason = parse_chat(&key).unwrap_err().to_string();
        assert_eq!(blocked(&reason), Some(Blocked::Key), "{reason}");
        let busy =
            serde_json::json!({"error": {"code": 502, "message": "Provider returned error"}});
        assert_eq!(blocked(&parse_chat(&busy).unwrap_err().to_string()), None);
        assert_eq!(
            blocked("could not reach OpenRouter: curl: (28) Operation timed out"),
            None
        );
    }

    #[test]
    fn truncation_is_a_failure() {
        let reply = serde_json::json!({
            "choices": [{"finish_reason": "length", "message": {"content": "{\"edits\": ["}}]
        });
        assert!(parse_chat(&reply).is_err());
    }
}
