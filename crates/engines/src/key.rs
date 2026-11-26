//! API-key engines: one adapter, four request shapes. Every run needs a source URL, or it is
//! `NoSearch` and is never stored.

use crate::{
    Account, AccountMode, Citation, Completion, CompletionRequest, Engine, EngineAnswer,
    EngineError, EngineRequest, Provider,
    http::{self, post_json},
    prices,
};
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct KeyEngine {
    provider: Provider,
    http: reqwest::Client,
    base_url: String,
}

/// The model a backend uses when its `model` column is empty.
pub fn default_model(provider: Provider) -> &'static str {
    match provider {
        Provider::Openai => "gpt-5-mini",
        Provider::Claude => "claude-sonnet-4-5",
        Provider::Perplexity => "sonar",
        Provider::Gemini => "gemini-2.5-flash",
    }
}

fn default_base_url(provider: Provider) -> &'static str {
    match provider {
        Provider::Openai => "https://api.openai.com",
        Provider::Claude => "https://api.anthropic.com",
        Provider::Perplexity => "https://api.perplexity.ai",
        Provider::Gemini => "https://generativelanguage.googleapis.com",
    }
}

impl KeyEngine {
    /// `base_url`: tests and proxies point the engine elsewhere.
    pub fn new(provider: Provider, base_url: Option<String>) -> Self {
        Self {
            provider,
            http: http::client(),
            base_url: base_url
                .unwrap_or_else(|| default_base_url(provider).to_owned())
                .trim_end_matches('/')
                .to_owned(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }
}

#[async_trait]
impl Engine for KeyEngine {
    fn provider(&self) -> Provider {
        self.provider
    }

    fn mode(&self) -> AccountMode {
        AccountMode::Key
    }

    async fn run(
        &self,
        account: &Account,
        request: &EngineRequest,
    ) -> Result<EngineAnswer, EngineError> {
        let key = account
            .secret
            .as_ref()
            .ok_or(EngineError::NeedsKey)?
            .expose()
            .to_owned();
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| default_model(self.provider).to_owned());
        let prompt = &request.prompt;
        match self.provider {
            Provider::Openai => {
                let mut tool = json!({"type": "web_search"});
                if let Some(country) = request.locale.split('-').nth(1) {
                    tool["user_location"] = json!({"type": "approximate", "country": country});
                }
                let body = json!({
                    "model": model, "input": prompt, "store": false,
                    "tools": [tool], "tool_choice": "required",
                    "include": ["web_search_call.action.sources"],
                });
                let v = post_json(
                    &self.http,
                    &self.url("/v1/responses"),
                    &[("authorization", format!("Bearer {key}"))],
                    &body,
                )
                .await?;
                parse_openai_response(&v, &model)
            }
            Provider::Claude => {
                let body = json!({
                    "model": model, "max_tokens": 2048,
                    "tools": [{"type": "web_search_20250305", "name": "web_search", "max_uses": 5}],
                    "messages": [{"role": "user", "content": prompt}],
                });
                let v = post_json(
                    &self.http,
                    &self.url("/v1/messages"),
                    &claude_headers(&key),
                    &body,
                )
                .await?;
                parse_claude_response(&v, &model)
            }
            Provider::Perplexity => {
                let body =
                    json!({"model": model, "messages": [{"role": "user", "content": prompt}]});
                let v = post_json(
                    &self.http,
                    &self.url("/chat/completions"),
                    &[("authorization", format!("Bearer {key}"))],
                    &body,
                )
                .await?;
                parse_perplexity_response(&v, &model)
            }
            Provider::Gemini => {
                let body = json!({
                    "contents": [{"parts": [{"text": prompt}]}],
                    "tools": [{"google_search": {}}],
                });
                let url = self.url(&format!("/v1beta/models/{model}:generateContent"));
                let v = post_json(&self.http, &url, &[("x-goog-api-key", key)], &body).await?;
                parse_gemini_response(&v, &model)
            }
        }
    }

    async fn complete(
        &self,
        account: &Account,
        request: &CompletionRequest,
    ) -> Result<Completion, EngineError> {
        let key = account
            .secret
            .as_ref()
            .ok_or(EngineError::NeedsKey)?
            .expose()
            .to_owned();
        match self.provider {
            Provider::Openai => {
                let body = json!({
                    "model": request.model, "instructions": request.system,
                    "input": request.user, "store": false,
                });
                let v = post_json(
                    &self.http,
                    &self.url("/v1/responses"),
                    &[("authorization", format!("Bearer {key}"))],
                    &body,
                )
                .await?;
                parse_openai_completion(&v, &request.model)
            }
            Provider::Claude => {
                let body = json!({
                    "model": request.model, "max_tokens": 1024, "system": request.system,
                    "messages": [{"role": "user", "content": request.user}],
                });
                let v = post_json(
                    &self.http,
                    &self.url("/v1/messages"),
                    &claude_headers(&key),
                    &body,
                )
                .await?;
                let text = claude_text(&v);
                let (tokens_in, tokens_out) = usage(&v["usage"], "input_tokens", "output_tokens");
                Ok(completion(
                    text,
                    &v["model"],
                    &request.model,
                    tokens_in,
                    tokens_out,
                ))
            }
            _ => Err(EngineError::Unavailable(
                "this provider cannot run plain model calls".into(),
            )),
        }
    }
}

fn claude_headers(key: &str) -> [(&'static str, String); 2] {
    [
        ("x-api-key", key.to_owned()),
        ("anthropic-version", "2023-06-01".to_owned()),
    ]
}

fn usage(v: &Value, input: &str, output: &str) -> (i32, i32) {
    let n = |k: &str| i32::try_from(v[k].as_i64().unwrap_or(0)).unwrap_or(i32::MAX);
    (n(input), n(output))
}

fn completion(
    text: String,
    reported: &Value,
    asked: &str,
    tokens_in: i32,
    tokens_out: i32,
) -> Completion {
    let model = reported.as_str().unwrap_or(asked).to_owned();
    Completion {
        cost_cents: prices::cost_cents(&model, tokens_in, tokens_out),
        text,
        tokens_in,
        tokens_out,
        model,
    }
}

/// Add a source once. A repeated URL keeps the first title.
fn push(cites: &mut Vec<Citation>, url: Option<&str>, title: Option<&str>) {
    let Some(url) = url.filter(|u| u.starts_with("http")) else {
        return;
    };
    if !cites.iter().any(|c| c.url == url) {
        cites.push(Citation {
            url: url.to_owned(),
            title: title.filter(|t| !t.is_empty()).map(str::to_owned),
        });
    }
}

fn finish(
    text: String,
    citations: Vec<Citation>,
    model: String,
    tokens: (i32, i32),
    raw: &Value,
) -> Result<EngineAnswer, EngineError> {
    if citations.is_empty() || text.trim().is_empty() {
        return Err(EngineError::NoSearch);
    }
    Ok(EngineAnswer {
        cost_cents: prices::cost_cents(&model, tokens.0, tokens.1),
        text,
        citations,
        model,
        tokens_in: tokens.0,
        tokens_out: tokens.1,
        raw: raw.clone(),
    })
}

fn openai_text(v: &Value) -> String {
    v["output"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["type"] == "message")
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter(|c| c["type"] == "output_text")
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("")
}

/// A Responses API object, from a key call or from the `response.completed` stream event.
pub(crate) fn parse_openai_response(v: &Value, asked: &str) -> Result<EngineAnswer, EngineError> {
    if let Some(status) = v["status"].as_str()
        && status != "completed"
    {
        let code = v["error"]["code"].as_str().unwrap_or(status);
        return Err(http::map_status(
            if code.contains("limit") { 429 } else { 502 },
            None,
            &v["error"].to_string(),
        ));
    }
    let mut cites = Vec::new();
    for item in v["output"].as_array().into_iter().flatten() {
        if item["type"] == "web_search_call" {
            for s in item["action"]["sources"].as_array().into_iter().flatten() {
                push(&mut cites, s["url"].as_str(), s["title"].as_str());
            }
        }
        for c in item["content"].as_array().into_iter().flatten() {
            for a in c["annotations"].as_array().into_iter().flatten() {
                if a["type"] == "url_citation" {
                    push(&mut cites, a["url"].as_str(), a["title"].as_str());
                }
            }
        }
    }
    let model = v["model"].as_str().unwrap_or(asked).to_owned();
    finish(
        openai_text(v),
        cites,
        model,
        usage(&v["usage"], "input_tokens", "output_tokens"),
        v,
    )
}

pub(crate) fn parse_openai_completion(v: &Value, asked: &str) -> Result<Completion, EngineError> {
    if let Some(status) = v["status"].as_str()
        && status != "completed"
    {
        return Err(EngineError::Unavailable(format!("response {status}")));
    }
    let (tokens_in, tokens_out) = usage(&v["usage"], "input_tokens", "output_tokens");
    Ok(completion(
        openai_text(v),
        &v["model"],
        asked,
        tokens_in,
        tokens_out,
    ))
}

fn claude_text(v: &Value) -> String {
    v["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("")
}

fn parse_claude_response(v: &Value, asked: &str) -> Result<EngineAnswer, EngineError> {
    let mut cites = Vec::new();
    for block in v["content"].as_array().into_iter().flatten() {
        if block["type"] == "web_search_tool_result" {
            for r in block["content"].as_array().into_iter().flatten() {
                push(&mut cites, r["url"].as_str(), r["title"].as_str());
            }
        }
        for c in block["citations"].as_array().into_iter().flatten() {
            push(&mut cites, c["url"].as_str(), c["title"].as_str());
        }
    }
    let model = v["model"].as_str().unwrap_or(asked).to_owned();
    finish(
        claude_text(v),
        cites,
        model,
        usage(&v["usage"], "input_tokens", "output_tokens"),
        v,
    )
}

fn parse_perplexity_response(v: &Value, asked: &str) -> Result<EngineAnswer, EngineError> {
    let mut cites = Vec::new();
    for r in v["search_results"].as_array().into_iter().flatten() {
        push(&mut cites, r["url"].as_str(), r["title"].as_str());
    }
    for url in v["citations"].as_array().into_iter().flatten() {
        push(&mut cites, url.as_str(), None);
    }
    let text = v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    let model = v["model"].as_str().unwrap_or(asked).to_owned();
    finish(
        text,
        cites,
        model,
        usage(&v["usage"], "prompt_tokens", "completion_tokens"),
        v,
    )
}

fn parse_gemini_response(v: &Value, asked: &str) -> Result<EngineAnswer, EngineError> {
    let candidate = &v["candidates"][0];
    let text: String = candidate["content"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["text"].as_str())
        .collect();
    let mut cites = Vec::new();
    for chunk in candidate["groundingMetadata"]["groundingChunks"]
        .as_array()
        .into_iter()
        .flatten()
    {
        push(
            &mut cites,
            chunk["web"]["uri"].as_str(),
            chunk["web"]["title"].as_str(),
        );
    }
    let model = v["modelVersion"].as_str().unwrap_or(asked).to_owned();
    finish(
        text,
        cites,
        model,
        usage(
            &v["usageMetadata"],
            "promptTokenCount",
            "candidatesTokenCount",
        ),
        v,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_sources_come_from_the_search_call_and_the_annotations_without_repeats() {
        let v = json!({
            "status": "completed", "model": "gpt-5-mini-2025-08-07",
            "usage": {"input_tokens": 100, "output_tokens": 50},
            "output": [
                {"type": "reasoning"},
                {"type": "web_search_call", "action": {"sources": [{"url": "https://a.example/x"}]}},
                {"type": "message", "content": [{"type": "output_text", "text": "Acme is great.",
                    "annotations": [{"type": "url_citation", "url": "https://a.example/x", "title": "A"},
                                    {"type": "url_citation", "url": "https://b.example", "title": "B"}]}]}
            ]
        });
        let a = parse_openai_response(&v, "gpt-5-mini").unwrap();
        assert_eq!(a.text, "Acme is great.");
        assert_eq!(a.citations.len(), 2);
        assert_eq!((a.tokens_in, a.tokens_out), (100, 50));
        assert!(a.cost_cents.unwrap() > 0.0);
    }

    #[test]
    fn an_answer_without_any_source_url_is_no_search() {
        let v = json!({"status": "completed", "output": [
            {"type": "message", "content": [{"type": "output_text", "text": "From memory.", "annotations": []}]}]});
        assert!(matches!(
            parse_openai_response(&v, "m"),
            Err(EngineError::NoSearch)
        ));
    }

    #[test]
    fn a_failed_response_with_a_limit_code_is_a_rate_limit() {
        let v = json!({"status": "failed", "error": {"code": "subscription_sharing_usage_limit_exceeded"}});
        assert!(matches!(
            parse_openai_response(&v, "m"),
            Err(EngineError::RateLimited {
                usage_limit: true,
                ..
            })
        ));
    }

    #[test]
    fn claude_reads_search_results_and_text_citations() {
        let v = json!({"model": "claude-sonnet-4-5", "usage": {"input_tokens": 9, "output_tokens": 3},
        "content": [
            {"type": "web_search_tool_result", "content": [{"type": "web_search_result", "url": "https://a.example", "title": "A"}]},
            {"type": "text", "text": "Use Acme.", "citations": [{"type": "web_search_result_location", "url": "https://b.example", "title": "B"}]}
        ]});
        let a = parse_claude_response(&v, "m").unwrap();
        assert_eq!(a.citations.len(), 2);
        assert_eq!(a.text, "Use Acme.");
    }

    #[test]
    fn perplexity_and_gemini_find_their_sources() {
        let p = json!({"choices": [{"message": {"content": "Answer"}}], "citations": ["https://a.example"],
            "search_results": [{"url": "https://a.example", "title": "A"}], "usage": {"prompt_tokens": 4, "completion_tokens": 2}});
        let a = parse_perplexity_response(&p, "sonar").unwrap();
        assert_eq!((a.citations.len(), a.tokens_in), (1, 4));
        let g = json!({"candidates": [{"content": {"parts": [{"text": "Hi"}]},
            "groundingMetadata": {"groundingChunks": [{"web": {"uri": "https://g.example/r", "title": "g.example"}}]}}],
            "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 1}});
        let a = parse_gemini_response(&g, "gemini-2.5-flash").unwrap();
        assert_eq!((a.text.as_str(), a.citations.len()), ("Hi", 1));
    }
}
