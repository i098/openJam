//! Engine access: the `Engine` trait, adapters for each provider, secret storage, account
//! rotation and the router that ties them together.
//!
//! A workspace has one `engine_backends` row per provider and any number of `engine_accounts`
//! under it: an API key (`mode = key`) or a subscription login (`mode = plan`).

pub mod claude;
mod http;
pub mod key;
pub mod pool;
pub mod prices;
pub mod router;
pub mod secrets;
pub mod siwc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fmt, str::FromStr};
use uuid::Uuid;

pub use router::{Backend, Completion, CompletionRequest, Routed, Router};
pub use secrets::Secret;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Openai,
    Gemini,
    Perplexity,
    Claude,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Gemini => "gemini",
            Self::Perplexity => "perplexity",
            Self::Claude => "claude",
        }
    }

    /// Only OpenAI and Claude have a plan route.
    pub fn has_plan(self) -> bool {
        matches!(self, Self::Openai | Self::Claude)
    }
}

impl FromStr for Provider {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "openai" => Ok(Self::Openai),
            "gemini" => Ok(Self::Gemini),
            "perplexity" => Ok(Self::Perplexity),
            "claude" => Ok(Self::Claude),
            other => Err(format!("unknown provider {other}")),
        }
    }
}

/// How an account reaches the provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountMode {
    /// An API key owned by the workspace.
    Key,
    /// A subscription sign-in.
    Plan,
}

impl AccountMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Plan => "plan",
        }
    }

    /// The value stored on an answer: `api_key` or `plan`.
    pub fn route(self) -> &'static str {
        match self {
            Self::Key => "api_key",
            Self::Plan => "plan",
        }
    }
}

impl FromStr for AccountMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "key" => Ok(Self::Key),
            "plan" => Ok(Self::Plan),
            other => Err(format!("unknown account mode {other}")),
        }
    }
}

/// One `engine_accounts` row. `secret` is the decrypted API key (mode key); a plan account reads
/// its own tokens, so it has none. `account_ref` is the OpenAI client id or the Claude folder.
#[derive(Clone, Debug)]
pub struct Account {
    pub id: Uuid,
    pub backend_id: Uuid,
    pub provider: Provider,
    pub mode: AccountMode,
    pub label: String,
    pub account_ref: Option<String>,
    pub secret: Option<Secret>,
}

#[derive(Clone, Debug)]
pub struct EngineRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub locale: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    pub url: String,
    pub title: Option<String>,
}

#[derive(Clone, Debug)]
pub struct EngineAnswer {
    pub text: String,
    pub citations: Vec<Citation>,
    pub model: String,
    pub tokens_in: i32,
    pub tokens_out: i32,
    /// `None`: the model has no price in `prices`.
    pub cost_cents: Option<f64>,
    /// The provider response, stored with the answer.
    pub raw: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// HTTP 429 or a plan limit. `usage_limit`: the whole account is out until `retry_at`.
    #[error("rate limited")]
    RateLimited {
        retry_at: Option<DateTime<Utc>>,
        usage_limit: bool,
    },
    /// No usable plan sign-in: none exists, or the login ended.
    #[error("sign in to use the plan")]
    NeedsLogin,
    #[error("this plan account cannot share plan use")]
    NotEligible,
    #[error("add an API key for this engine")]
    NeedsKey,
    /// The provider rejected the credential.
    #[error("credential rejected")]
    Unauthorized,
    /// The answer used no web search. It is never stored.
    #[error("the answer used no web search")]
    NoSearch,
    #[error("this route cannot search the web")]
    SearchUnsupported,
    #[error("provider call timed out")]
    Timeout,
    #[error("{0}")]
    Unavailable(String),
    #[error("spend cap reached")]
    CapReached,
}

impl EngineError {
    /// The code stored on the scan call and shown by the API.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NeedsLogin | Self::NotEligible => "engine_needs_login",
            Self::NeedsKey | Self::Unauthorized => "engine_needs_key",
            Self::RateLimited { .. } => "engine_limited",
            Self::NoSearch | Self::SearchUnsupported => "engine_no_search",
            Self::Timeout | Self::Unavailable(_) => "engine_unavailable",
            Self::CapReached => "cap_reached",
        }
    }
}

impl From<sqlx::Error> for EngineError {
    fn from(e: sqlx::Error) -> Self {
        Self::Unavailable(format!("database: {e}"))
    }
}

#[async_trait]
pub trait Engine: Send + Sync {
    fn provider(&self) -> Provider;
    fn mode(&self) -> AccountMode;
    /// Ask one prompt through one account, with web search.
    async fn run(
        &self,
        account: &Account,
        request: &EngineRequest,
    ) -> Result<EngineAnswer, EngineError>;
    /// A plain model call with no search, for the judge. Not every route supports it.
    async fn complete(
        &self,
        _account: &Account,
        _request: &CompletionRequest,
    ) -> Result<Completion, EngineError> {
        Err(EngineError::Unavailable(
            "this route cannot run plain model calls".into(),
        ))
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_modes_serialize_as_plan_or_key() {
        assert_eq!(
            serde_json::to_value(AccountMode::Plan).unwrap(),
            serde_json::json!("plan")
        );
        assert_eq!(
            serde_json::from_value::<AccountMode>(serde_json::json!("key")).unwrap(),
            AccountMode::Key
        );
    }

    #[test]
    fn error_codes_follow_the_design_table() {
        assert_eq!(
            EngineError::RateLimited {
                retry_at: None,
                usage_limit: true
            }
            .code(),
            "engine_limited"
        );
        assert_eq!(EngineError::NoSearch.code(), "engine_no_search");
        assert_eq!(EngineError::Timeout.code(), "engine_unavailable");
    }
}
