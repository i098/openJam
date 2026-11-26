//! Parse the `stream-json` output of the Claude Code CLI.
//!
//! The exact event shape is not fixed yet, so every read is defensive.

use std::collections::HashSet;

use chrono::{DateTime, SecondsFormat};
use serde::Serialize;
use serde_json::Value;

const LOGGED_OUT: &[&str] = &[
    "not logged in",
    "please run /login",
    "invalid api key",
    "authentication",
];
const LIMITED: &[&str] = &["usage limit", "limit reached", "rate limit", "5-hour limit"];
/// Marker words are only checked in short or error answers, so a real answer about
/// "authentication" or "rate limits" stays an answer.
const SHORT_TEXT: usize = 500;

#[derive(Debug, Serialize, PartialEq)]
pub struct Source {
    pub url: String,
    pub title: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Limit {
    pub reset_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Usage {
    pub tokens_in: u64,
    pub tokens_out: u64,
}

#[derive(Debug, Serialize)]
pub struct Reply {
    pub text: String,
    pub sources: Vec<Source>,
    pub search_used: bool,
    pub limit: Option<Limit>,
    pub logged_out: bool,
    pub usage: Option<Usage>,
    pub model: Option<String>,
}

/// What the run gave, when it is not a normal reply.
#[derive(Debug, PartialEq)]
pub enum Failure {
    NoResult,
    MaxTurns,
}

#[derive(Default)]
pub struct Stream {
    search_ids: HashSet<String>,
    search_used: bool,
    sources: Sources,
    model: Option<String>,
    result: Option<Value>,
}

impl Stream {
    /// Read one stdout line. Lines that are not JSON are ignored.
    pub fn line(&mut self, line: &str) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match event["type"].as_str() {
            Some("assistant") => {
                if let Some(model) = event["message"]["model"].as_str() {
                    self.model = Some(model.to_string());
                }
                for block in blocks(&event) {
                    match block["type"].as_str() {
                        Some("tool_use" | "server_tool_use") if is_search(block) => {
                            self.search_used = true;
                            if let Some(id) = block["id"].as_str() {
                                self.search_ids.insert(id.to_string());
                            }
                        }
                        Some("web_search_tool_result") => {
                            self.search_used = true;
                            self.sources.collect(block);
                        }
                        _ => {}
                    }
                }
            }
            Some("user") => {
                let from_search = blocks(&event).iter().any(|block| {
                    block["type"] == "tool_result"
                        && block["tool_use_id"]
                            .as_str()
                            .is_some_and(|id| self.search_ids.contains(id))
                });
                if from_search {
                    // The whole event: the tool_result block and any `tool_use_result` copy.
                    self.sources.collect(&event);
                }
            }
            Some("result") => self.result = Some(event),
            _ => {}
        }
    }

    pub fn finish(self) -> Result<Reply, Failure> {
        let result = self.result.ok_or(Failure::NoResult)?;
        if result["subtype"] == "error_max_turns" {
            return Err(Failure::MaxTurns);
        }
        let text = result["result"].as_str().unwrap_or_default();
        let is_error = result["is_error"].as_bool().unwrap_or(false);
        let lower = text.to_lowercase();
        let check = is_error || text.len() < SHORT_TEXT;
        let logged_out = check && LOGGED_OUT.iter().any(|m| lower.contains(m));
        let limited =
            !logged_out && (is_error || (check && LIMITED.iter().any(|m| lower.contains(m))));
        let usage = result["usage"].is_object().then(|| Usage {
            tokens_in: result["usage"]["input_tokens"].as_u64().unwrap_or(0),
            tokens_out: result["usage"]["output_tokens"].as_u64().unwrap_or(0),
        });
        let model = result["modelUsage"]
            .as_object()
            .and_then(|m| m.keys().next().cloned())
            .or_else(|| result["model"].as_str().map(str::to_string))
            .or(self.model);
        Ok(Reply {
            text: if limited || logged_out {
                String::new()
            } else {
                text.to_string()
            },
            sources: self.sources.list,
            search_used: self.search_used,
            limit: limited.then(|| Limit {
                reset_at: reset_at(text),
            }),
            logged_out,
            usage,
            model,
        })
    }
}

fn blocks(event: &Value) -> &[Value] {
    event["message"]["content"]
        .as_array()
        .map_or(&[], Vec::as_slice)
}

fn is_search(block: &Value) -> bool {
    matches!(block["name"].as_str(), Some("WebSearch" | "web_search"))
}

#[derive(Default)]
struct Sources {
    list: Vec<Source>,
    seen: HashSet<String>,
}

impl Sources {
    /// Take URLs with titles first, then bare URLs from the text. The first one of a URL wins.
    fn collect(&mut self, value: &Value) {
        self.titled(value);
        self.bare(value);
    }

    fn add(&mut self, url: &str, title: Option<&str>) {
        if self.seen.insert(url.to_string()) {
            self.list.push(Source {
                url: url.to_string(),
                title: title.map(str::to_string),
            });
        }
    }

    fn titled(&mut self, value: &Value) {
        match value {
            Value::Object(map) => {
                if let Some(url) = map
                    .get("url")
                    .and_then(Value::as_str)
                    .filter(|u| is_http(u))
                {
                    self.add(url, map.get("title").and_then(Value::as_str));
                }
                map.values().for_each(|v| self.titled(v));
            }
            Value::Array(items) => items.iter().for_each(|v| self.titled(v)),
            Value::String(text) => embedded_json(text).iter().for_each(|v| self.titled(v)),
            _ => {}
        }
    }

    fn bare(&mut self, value: &Value) {
        match value {
            Value::Object(map) => map.values().for_each(|v| self.bare(v)),
            Value::Array(items) => items.iter().for_each(|v| self.bare(v)),
            Value::String(text) => {
                for url in scan_urls(text) {
                    self.add(url, None);
                }
            }
            _ => {}
        }
    }
}

fn is_http(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// JSON values written inside a text, such as `Links: [{"title":..,"url":..}]`.
fn embedded_json(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(offset) = text[at..].find(['{', '[']) {
        let start = at + offset;
        let mut values = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
        match values.next() {
            Some(Ok(value)) => {
                at = start + values.byte_offset();
                out.push(value);
            }
            _ => at = start + 1,
        }
    }
    out
}

fn scan_urls(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("http") {
        let tail = &rest[i..];
        if !is_http(tail) {
            rest = &tail[4..];
            continue;
        }
        let end = tail
            .find(|c: char| c.is_whitespace() || "\"'<>()[]{}`\\".contains(c))
            .unwrap_or(tail.len());
        let url = tail[..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
        if url.len() > "https://".len() {
            out.push(url);
        }
        rest = &tail[end..];
    }
    out
}

/// Find a reset time after the word "reset", or anywhere when the word is missing
/// (the CLI has printed `usage limit reached|<epoch>`).
fn reset_at(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let tail = lower.find("reset").map_or(text, |i| &text[i..]);
    tail.split(|c: char| !(c.is_ascii_alphanumeric() || "+-:.".contains(c)))
        .find_map(|token| {
            let token = token.trim_end_matches('.');
            let time = match token.len() {
                10 | 13 if token.bytes().all(|b| b.is_ascii_digit()) => {
                    let n: i64 = token.parse().ok()?;
                    let secs = if token.len() == 13 { n / 1000 } else { n };
                    DateTime::from_timestamp(secs, 0)?
                }
                _ => DateTime::parse_from_rfc3339(token).ok()?.to_utc(),
            };
            Some(time.to_rfc3339_opts(SecondsFormat::Secs, true))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_from_text_and_json() {
        let text = r#"Results: Links: [{"title":"A","url":"https://a.example/x"}] see https://b.example/y. and https://a.example/x"#;
        let mut s = Sources::default();
        s.collect(&Value::String(text.into()));
        assert_eq!(
            s.list,
            vec![
                Source {
                    url: "https://a.example/x".into(),
                    title: Some("A".into())
                },
                Source {
                    url: "https://b.example/y".into(),
                    title: None
                },
            ]
        );
    }

    #[test]
    fn reset_times() {
        assert_eq!(
            reset_at("Claude AI usage limit reached|1767225600").as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        assert_eq!(
            reset_at("Limit reached. Resets at 2026-01-01T05:00:00+02:00.").as_deref(),
            Some("2026-01-01T03:00:00Z")
        );
        assert_eq!(reset_at("5-hour limit reached, resets 3pm"), None);
    }
}
