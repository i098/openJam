//! The router: one prompt in, one answer out, through the plan accounts first and the key accounts
//! next. It owns the rules for a limit, a lost login, a bad key, a retry and the spend cap.

use crate::{
    Account, AccountMode, Engine, EngineAnswer, EngineError, EngineRequest, Provider,
    pool::{BlockReason, Pick, Pool, Rotation},
};
use futures_util::future::BoxFuture;
use sqlx::{PgPool, Row};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

/// Plan runs at one time for each OpenAI plan account in this process.
/// shortcut: with several worker processes the limit is per process. Use a database counter then.
const PLAN_CONCURRENCY: usize = 2;
/// One retry on the same account after a timeout or a 5xx.
const ATTEMPTS_PER_ACCOUNT: usize = 2;

/// One `engine_backends` row.
#[derive(Clone, Debug)]
pub struct Backend {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub provider: Provider,
    /// Model for key accounts. `None`: the provider default.
    pub model: Option<String>,
    pub plan_model: Option<String>,
    pub rotation: Rotation,
    pub key_fallback: bool,
    pub enabled: bool,
}

impl Backend {
    pub async fn load(db: &PgPool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, workspace_id, provider, model, plan_model, rotation, key_fallback, enabled
             FROM engine_backends WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(db)
        .await?;
        Ok(row.map(|r| Self::from_row(&r)))
    }

    pub async fn for_workspace(
        db: &PgPool,
        workspace_id: Uuid,
        provider: Provider,
    ) -> Result<Option<Self>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, workspace_id, provider, model, plan_model, rotation, key_fallback, enabled
             FROM engine_backends WHERE workspace_id = $1 AND provider = $2",
        )
        .bind(workspace_id)
        .bind(provider.as_str())
        .fetch_optional(db)
        .await?;
        Ok(row.map(|r| Self::from_row(&r)))
    }

    fn from_row(r: &sqlx::postgres::PgRow) -> Self {
        Self {
            id: r.get("id"),
            workspace_id: r.get("workspace_id"),
            provider: r
                .get::<String, _>("provider")
                .parse()
                .expect("the provider column is checked"),
            model: r.get("model"),
            plan_model: r.get("plan_model"),
            rotation: if r.get::<String, _>("rotation") == "standby" {
                Rotation::Standby
            } else {
                Rotation::Rotate
            },
            key_fallback: r.get("key_fallback"),
            enabled: r.get("enabled"),
        }
    }
}

/// A plain model call, for the judge.
#[derive(Clone, Debug)]
pub struct CompletionRequest {
    pub model: String,
    pub system: String,
    pub user: String,
}

#[derive(Clone, Debug)]
pub struct Completion {
    pub text: String,
    pub tokens_in: i32,
    pub tokens_out: i32,
    pub cost_cents: Option<f64>,
    pub model: String,
}

/// A result and the account that made it.
#[derive(Debug)]
pub struct Routed<T> {
    pub value: T,
    pub mode: AccountMode,
    pub account_id: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelTier {
    Small,
    Big,
}

/// The judge models. The small one runs first; the big one decides hard cases.
#[derive(Clone, Debug)]
pub struct JudgeModels {
    pub openai_small: String,
    pub openai_big: String,
    pub claude_small: String,
    pub claude_big: String,
}

impl Default for JudgeModels {
    fn default() -> Self {
        Self {
            openai_small: "gpt-5-nano".into(),
            openai_big: "gpt-5".into(),
            claude_small: "claude-haiku-4-5".into(),
            claude_big: "claude-sonnet-4-5".into(),
        }
    }
}

impl JudgeModels {
    pub fn from_env() -> Self {
        let pick = |name: &str, default: String| {
            std::env::var(name)
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or(default)
        };
        let d = Self::default();
        Self {
            openai_small: pick("OPENJAM_JUDGE_SMALL_MODEL", d.openai_small),
            openai_big: pick("OPENJAM_JUDGE_BIG_MODEL", d.openai_big),
            claude_small: pick("OPENJAM_JUDGE_CLAUDE_SMALL_MODEL", d.claude_small),
            claude_big: pick("OPENJAM_JUDGE_CLAUDE_BIG_MODEL", d.claude_big),
        }
    }

    fn model(&self, provider: Provider, tier: ModelTier) -> &str {
        match (provider, tier) {
            (Provider::Claude, ModelTier::Small) => &self.claude_small,
            (Provider::Claude, ModelTier::Big) => &self.claude_big,
            (_, ModelTier::Small) => &self.openai_small,
            (_, ModelTier::Big) => &self.openai_big,
        }
    }
}

enum Tier<T> {
    Done(Routed<T>),
    /// No account of this mode could run the call.
    Empty,
    Failed(EngineError),
}

pub struct Router {
    pool: Pool,
    engines: Vec<Arc<dyn Engine>>,
    judge_models: JudgeModels,
    permits: std::sync::Mutex<HashMap<Uuid, Arc<Semaphore>>>,
}

type Op<'a, T> =
    &'a (dyn Fn(Arc<dyn Engine>, Account) -> BoxFuture<'static, Result<T, EngineError>> + Sync);

impl Router {
    pub fn new(pool: Pool, engines: Vec<Arc<dyn Engine>>, judge_models: JudgeModels) -> Self {
        Self {
            pool,
            engines,
            judge_models,
            permits: Default::default(),
        }
    }

    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Ask one prompt. Plan accounts run first; the key accounts run when the plan tier has none,
    /// or when it failed and the backend allows the key fallback.
    pub async fn ask(
        &self,
        backend: &Backend,
        request: &EngineRequest,
    ) -> Result<Routed<EngineAnswer>, EngineError> {
        let (key_model, plan_model) = (backend.model.clone(), backend.plan_model.clone());
        let request = request.clone();
        let op = move |engine: Arc<dyn Engine>,
                       account: Account|
              -> BoxFuture<'static, Result<EngineAnswer, EngineError>> {
            let mut r = request.clone();
            r.model = if account.mode == AccountMode::Plan {
                plan_model.clone()
            } else {
                key_model.clone()
            };
            Box::pin(async move { engine.run(&account, &r).await })
        };
        self.drive(backend, backend.model.as_deref().unwrap_or("default"), &op)
            .await
    }

    /// A plain model call for the judge. It uses the workspace's OpenAI accounts (plan first), then
    /// its Claude key accounts. With none of them: `NeedsKey`.
    pub async fn complete(
        &self,
        workspace_id: Uuid,
        tier: ModelTier,
        system: &str,
        user: &str,
    ) -> Result<Routed<Completion>, EngineError> {
        let mut last = EngineError::NeedsKey;
        for provider in [Provider::Openai, Provider::Claude] {
            let Some(backend) =
                Backend::for_workspace(self.pool.db(), workspace_id, provider).await?
            else {
                continue;
            };
            if !backend.enabled {
                continue;
            }
            let req = CompletionRequest {
                model: self.judge_models.model(provider, tier).to_owned(),
                system: system.to_owned(),
                user: user.to_owned(),
            };
            let scope = format!("model:{}", req.model);
            let op = move |engine: Arc<dyn Engine>,
                           account: Account|
                  -> BoxFuture<'static, Result<Completion, EngineError>> {
                let req = req.clone();
                Box::pin(async move { engine.complete(&account, &req).await })
            };
            match self.drive(&backend, &scope["model:".len()..], &op).await {
                Ok(done) => return Ok(done),
                Err(EngineError::CapReached) => return Err(EngineError::CapReached),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    async fn drive<T: Send + 'static>(
        &self,
        backend: &Backend,
        model: &str,
        op: Op<'_, T>,
    ) -> Result<Routed<T>, EngineError> {
        let scope = format!("model:{model}");
        let mut tried = Vec::new();
        let mut failure = None;
        match self
            .tier(backend, AccountMode::Plan, &scope, op, &mut tried)
            .await?
        {
            Tier::Done(done) => return Ok(done),
            Tier::Failed(e) => failure = Some(e),
            Tier::Empty => {}
        }
        if !backend.key_fallback
            && let Some(failure) = failure
        {
            return Err(failure);
        }
        if self.spend_cap_reached(backend.workspace_id).await? {
            return Err(EngineError::CapReached);
        }
        match self
            .tier(backend, AccountMode::Key, &scope, op, &mut tried)
            .await?
        {
            Tier::Done(done) => Ok(done),
            Tier::Failed(e) => Err(e),
            Tier::Empty => Err(failure.unwrap_or(if backend.provider == Provider::Openai {
                EngineError::NeedsLogin
            } else {
                EngineError::NeedsKey
            })),
        }
    }

    async fn tier<T: Send + 'static>(
        &self,
        backend: &Backend,
        mode: AccountMode,
        scope: &str,
        op: Op<'_, T>,
        tried: &mut Vec<Uuid>,
    ) -> Result<Tier<T>, EngineError> {
        let mut last: Option<EngineError> = None;
        loop {
            let account = match self
                .pool
                .checkout(
                    backend.id,
                    backend.provider,
                    mode,
                    backend.rotation,
                    scope,
                    tried,
                )
                .await?
            {
                Pick::Account(a) => a,
                Pick::Blocked(until) => {
                    return Ok(Tier::Failed(EngineError::RateLimited {
                        retry_at: Some(until),
                        usage_limit: true,
                    }));
                }
                Pick::Empty => return Ok(last.map_or(Tier::Empty, Tier::Failed)),
            };
            tried.push(account.id);
            let Some(engine) = self
                .engines
                .iter()
                .find(|e| e.provider() == backend.provider && e.mode() == mode)
            else {
                return Ok(Tier::Failed(EngineError::Unavailable(format!(
                    "no {} adapter for {}",
                    mode.as_str(),
                    backend.provider
                ))));
            };
            let _permit = self.permit(&account).await;
            let mut result = Err(EngineError::Timeout);
            for _ in 0..ATTEMPTS_PER_ACCOUNT {
                result = op(engine.clone(), account.clone()).await;
                if !matches!(
                    result,
                    Err(EngineError::Timeout | EngineError::Unavailable(_))
                ) {
                    break;
                }
            }
            let error = match result {
                Ok(value) => {
                    self.pool.report_ok(account.id).await?;
                    return Ok(Tier::Done(Routed {
                        value,
                        mode,
                        account_id: account.id,
                    }));
                }
                Err(e) => e,
            };
            tracing::warn!(account = %account.label, code = error.code(), "engine call failed");
            match &error {
                EngineError::RateLimited {
                    retry_at,
                    usage_limit,
                } => {
                    let (block_scope, reason) = if *usage_limit {
                        ("", BlockReason::UsageLimit)
                    } else {
                        (scope, BlockReason::RateLimit)
                    };
                    self.pool
                        .block(account.id, block_scope, *retry_at, reason)
                        .await?;
                    // standby: a limit never moves the prompt to another account.
                    if backend.rotation == Rotation::Standby {
                        return Ok(Tier::Failed(error));
                    }
                }
                EngineError::NeedsLogin => self.pool.disable(account.id, "logged_out").await?,
                EngineError::NotEligible => {
                    self.pool.disable(account.id, "plan_not_eligible").await?
                }
                EngineError::Unauthorized if mode == AccountMode::Key => {
                    self.pool.disable(account.id, "invalid_key").await?;
                }
                EngineError::NoSearch
                | EngineError::SearchUnsupported
                | EngineError::NeedsKey
                | EngineError::CapReached => {
                    return Ok(Tier::Failed(error));
                }
                _ => {}
            }
            // A 401 on a plan run that survived a token refresh means the login is no good.
            last = Some(
                if mode == AccountMode::Plan && matches!(error, EngineError::Unauthorized) {
                    EngineError::NeedsLogin
                } else {
                    error
                },
            );
        }
    }

    async fn permit(&self, account: &Account) -> Option<OwnedSemaphorePermit> {
        if account.mode != AccountMode::Plan || account.provider != Provider::Openai {
            return None;
        }
        let semaphore = self
            .permits
            .lock()
            .expect("permit map lock")
            .entry(account.id)
            .or_insert_with(|| Arc::new(Semaphore::new(PLAN_CONCURRENCY)))
            .clone();
        semaphore.acquire_owned().await.ok()
    }

    /// Rule 4: key calls stop when this month's cost reaches the workspace cap. Plan calls cost nothing.
    async fn spend_cap_reached(&self, workspace_id: Uuid) -> Result<bool, EngineError> {
        Ok(sqlx::query_scalar(
            "SELECT COALESCE(w.spend_cap_cents_month IS NOT NULL AND
                 COALESCE((SELECT sum(u.cost_cents) FROM usage_ledger u
                           WHERE u.workspace_id = w.id
                             AND u.at >= date_trunc('month', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'), 0)
                 >= w.spend_cap_cents_month, false)
             FROM workspaces w WHERE w.id = $1",
        )
        .bind(workspace_id)
        .fetch_optional(self.pool.db())
        .await?
        .unwrap_or(false))
    }
}
