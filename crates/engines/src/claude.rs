//! Claude plan engine: asks the runner to run the unmodified Claude Code CLI under one login
//! folder. openJam holds no Claude token. The runner sees prompts and answers only.

use crate::{
    Account, AccountMode, Citation, Engine, EngineAnswer, EngineError, EngineRequest, Provider,
    http::{self, send_error},
    secrets::Secret,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

/// What the runner returns for `POST /v1/run`.
#[derive(Deserialize)]
struct RunReply {
    #[serde(default)]
    text: String,
    #[serde(default)]
    sources: Vec<Citation>,
    #[serde(default)]
    search_used: bool,
    limit: Option<Limit>,
    #[serde(default)]
    logged_out: bool,
    usage: Option<Usage>,
    model: Option<String>,
}

#[derive(Deserialize)]
struct Limit {
    reset_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct Usage {
    tokens_in: i32,
    tokens_out: i32,
}

pub struct ClaudePlanEngine {
    http: reqwest::Client,
    runner_url: String,
    secret: Secret,
}

impl ClaudePlanEngine {
    pub fn new(runner_url: &str, shared_secret: Secret) -> Self {
        Self {
            http: http::client(),
            runner_url: runner_url.trim_end_matches('/').to_owned(),
            secret: shared_secret,
        }
    }
}

#[async_trait]
impl Engine for ClaudePlanEngine {
    fn provider(&self) -> Provider {
        Provider::Claude
    }

    fn mode(&self) -> AccountMode {
        AccountMode::Plan
    }

    async fn run(
        &self,
        account: &Account,
        request: &EngineRequest,
    ) -> Result<EngineAnswer, EngineError> {
        let folder = account
            .account_ref
            .as_deref()
            .ok_or(EngineError::NeedsLogin)?;
        let timeout = http::CALL_TIMEOUT.saturating_sub(std::time::Duration::from_secs(10));
        let resp = self
            .http
            .post(format!("{}/v1/run", self.runner_url))
            .bearer_auth(self.secret.expose())
            .json(&json!({
                "run_id": Uuid::new_v4(), "folder": folder, "prompt": request.prompt,
                "model": request.model, "timeout_ms": timeout.as_millis() as u64,
            }))
            .send()
            .await
            .map_err(send_error)?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(send_error)?;
        match status {
            200 => {}
            401 => {
                return Err(EngineError::Unavailable(
                    "the runner refused the shared secret".into(),
                ));
            }
            404 => return Err(EngineError::NeedsLogin),
            504 => return Err(EngineError::Timeout),
            // 503 runner_busy and every other failure: no block, the router tries again.
            _ => {
                return Err(EngineError::Unavailable(format!(
                    "runner answered {status}: {}",
                    text.chars().take(200).collect::<String>()
                )));
            }
        }
        reply_to_answer(
            serde_json::from_str(&text)
                .map_err(|e| EngineError::Unavailable(format!("bad runner reply: {e}")))?,
        )
    }
}

fn reply_to_answer(reply: RunReply) -> Result<EngineAnswer, EngineError> {
    if reply.logged_out {
        return Err(EngineError::NeedsLogin);
    }
    if let Some(limit) = reply.limit {
        return Err(EngineError::RateLimited {
            retry_at: limit.reset_at,
            usage_limit: true,
        });
    }
    if !reply.search_used || reply.sources.is_empty() || reply.text.trim().is_empty() {
        return Err(EngineError::NoSearch);
    }
    let usage = reply.usage.unwrap_or(Usage {
        tokens_in: 0,
        tokens_out: 0,
    });
    let model = reply.model.unwrap_or_else(|| "claude".into());
    Ok(EngineAnswer {
        raw: json!({"via": "runner", "search_used": true, "sources": reply.sources}),
        text: reply.text,
        citations: reply.sources,
        // The plan pays for the call. It costs no money.
        cost_cents: Some(0.0),
        tokens_in: usage.tokens_in,
        tokens_out: usage.tokens_out,
        model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(v: serde_json::Value) -> Result<EngineAnswer, EngineError> {
        reply_to_answer(serde_json::from_value(v).unwrap())
    }

    #[test]
    fn the_runner_reply_maps_to_the_engine_events() {
        let ok = reply(json!({"text": "Use Acme.", "sources": [{"url": "https://a.example", "title": null}], "search_used": true,
            "usage": {"tokens_in": 5, "tokens_out": 2}, "model": "claude-sonnet-4-5"})).unwrap();
        assert_eq!(
            (ok.citations.len(), ok.tokens_out, ok.cost_cents),
            (1, 2, Some(0.0))
        );
        assert!(matches!(
            reply(json!({"logged_out": true})),
            Err(EngineError::NeedsLogin)
        ));
        assert!(matches!(
            reply(json!({"limit": {"reset_at": "2026-11-27T10:00:00Z"}})),
            Err(EngineError::RateLimited {
                retry_at: Some(_),
                usage_limit: true
            })
        ));
        assert!(matches!(
            reply(json!({"text": "memory only", "search_used": false})),
            Err(EngineError::NoSearch)
        ));
    }
}
