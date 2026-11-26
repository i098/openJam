//! The `Engine` trait, provider and account types. Adapters and rotation come in the engine slices.
//!
//! A workspace holds one `engine_backends` row per provider, and any number of `engine_accounts`
//! under it: an API key (`mode = key`) or a subscription login (`mode = plan`).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Openai,
    Gemini,
    Perplexity,
    Claude,
}

/// How an account reaches the provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountMode {
    /// An API key owned by the workspace.
    Key,
    /// A subscription sign-in kept by the runner.
    Plan,
}

/// One `engine_accounts` row, without the credential. An adapter loads its own credential.
#[derive(Clone, Debug)]
pub struct Account {
    pub id: Uuid,
    pub backend_id: Uuid,
    pub mode: AccountMode,
    pub label: String,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct EngineRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub locale: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub cost_cents: f64,
    /// The provider response, stored with the answer.
    pub raw: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// HTTP 429, or a plan limit.
    #[error("rate limited")]
    RateLimited,
    /// A subscription login ended. The owner must sign in again.
    #[error("login expired")]
    LoginExpired,
    #[error("provider call timed out")]
    Timeout,
    #[error("{0}")]
    Provider(String),
}

#[async_trait]
pub trait Engine: Send + Sync {
    fn provider(&self) -> Provider;
    /// Ask one prompt through one account.
    async fn run(
        &self,
        account: &Account,
        request: &EngineRequest,
    ) -> Result<EngineAnswer, EngineError>;
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
}
