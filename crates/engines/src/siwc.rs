//! Sign in with ChatGPT (open-source flow): browser sign-in with PKCE, tokens kept sealed in
//! `engine_accounts`, single-flight refresh, and the plan engine that calls `/v1/responses` with
//! the plan token.
//!
//! The owner signs in on a machine whose browser reaches `http://127.0.0.1:<port>/auth/callback`,
//! or pastes the callback address by hand. Tokens stay on the machine that runs openJam.
//!
//! shortcut: the ID token is not checked against OpenAI's signing keys. It arrives straight from
//! the token endpoint over TLS, which OpenID Connect Core 3.1.3.7 accepts. Check `iss`, `aud`,
//! `exp` and `nonce` only. Add JWKS checks if an ID token ever comes from another path.

use crate::{
    Account, AccountMode, Completion, CompletionRequest, Engine, EngineAnswer, EngineError,
    EngineRequest, Provider,
    http::{self, map_status, send_error},
    key::{parse_openai_completion, parse_openai_response},
    pool::Pool,
    secrets::Secret,
};
use async_trait::async_trait;
use axum::{extract::State, http::StatusCode, response::Html, routing::get};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::{sync::Arc, time::Duration as StdDuration};
use tokio::sync::{OnceCell, mpsc};
use uuid::Uuid;

/// First registration: OpenAI issues a client id for this ChatGPT account and returns it.
const DYNAMIC_CLIENT: &str = "dynamic_agent_client";
const SCOPE: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const RESOURCE: &str = "https://api.openai.com/v1";
const NEEDED_SCOPE: &str = "chatgpt.tokens.use.direct";
const FLOW_MINUTES: i64 = 10;
/// Refresh this long before the access token ends.
const EXPIRY_MARGIN_SECS: i64 = 60;
const LEASE_SECS: i64 = 15;
/// The refresh call must end before its lease does.
const REFRESH_TIMEOUT: StdDuration = StdDuration::from_secs(10);
const TRANSIENT_RETRY_MINS: i64 = 5;
/// These refresh errors end the sign-in for good.
const DEAD_GRANTS: &[&str] = &[
    "invalid_grant",
    "invalid_refresh_token",
    "token_expired",
    "refresh_token_expired",
    "refresh_token_invalidated",
    "refresh_token_reused",
];

#[derive(Clone, Debug)]
pub struct SiwcConfig {
    pub auth_base: String,
    pub api_base: String,
    pub callback_port: u16,
}

impl SiwcConfig {
    pub fn from_env() -> Self {
        let get = |name: &str, default: &str| {
            std::env::var(name)
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| default.to_owned())
        };
        Self {
            auth_base: get("OPENAI_AUTH_BASE_URL", "https://auth.openai.com")
                .trim_end_matches('/')
                .to_owned(),
            api_base: get("OPENAI_API_BASE_URL", "https://api.openai.com")
                .trim_end_matches('/')
                .to_owned(),
            callback_port: get("OPENJAM_SIWC_CALLBACK_PORT", "1455")
                .parse()
                .unwrap_or(1455),
        }
    }

    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/auth/callback", self.callback_port)
    }
}

/// The three tokens of one sign-in, sealed together as JSON.
#[derive(Serialize, Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    id_token: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    id_token: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
}

#[derive(Debug)]
pub struct LoginStart {
    pub flow_id: Uuid,
    pub authorize_url: String,
}

#[derive(Debug)]
pub struct LoginDone {
    pub account_id: Uuid,
    pub label: String,
    pub email: Option<String>,
}

pub struct Siwc {
    pool: Pool,
    http: reqwest::Client,
    cfg: SiwcConfig,
    /// Names this process in refresh leases.
    owner: String,
}

impl Siwc {
    pub fn new(pool: Pool, cfg: SiwcConfig) -> Self {
        Self {
            pool,
            http: http::client(),
            cfg,
            owner: format!("{}-{}", std::process::id(), Uuid::new_v4().simple()),
        }
    }

    pub fn config(&self) -> &SiwcConfig {
        &self.cfg
    }

    /// Begin a sign-in. Open `authorize_url` in a browser. Use the same `label` again to sign in
    /// again to an account that already exists.
    pub async fn start_login(
        &self,
        workspace_id: Uuid,
        label: &str,
        background_consent: bool,
    ) -> Result<LoginStart, String> {
        let (verifier, challenge) = pkce();
        let (state, nonce) = (random_token(), random_token());
        let host_id: String = sqlx::query_scalar("SELECT ext_agent_host_id FROM engine_settings")
            .fetch_one(self.pool.db())
            .await
            .map_err(internal)?;
        let redirect_uri = self.cfg.redirect_uri();
        let flow_id: Uuid = sqlx::query_scalar(
            "INSERT INTO engine_login_flows
                 (workspace_id, label, state, nonce, code_verifier, redirect_uri, client_id, background_consent, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now() + make_interval(mins => $9))
             RETURNING id",
        )
        .bind(workspace_id)
        .bind(label)
        .bind(&state)
        .bind(&nonce)
        .bind(&verifier)
        .bind(&redirect_uri)
        .bind(DYNAMIC_CLIENT)
        .bind(background_consent)
        .bind(FLOW_MINUTES as f64)
        .fetch_one(self.pool.db())
        .await
        .map_err(internal)?;
        let mut url = url::Url::parse(&format!("{}/api/accounts/authorize", self.cfg.auth_base))
            .map_err(internal)?;
        url.query_pairs_mut()
            .append_pair("client_id", DYNAMIC_CLIENT)
            .append_pair("agent_name_hint", "openJam")
            .append_pair("ext_agent_host_id", &host_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", SCOPE)
            .append_pair("resource", RESOURCE)
            .append_pair("state", &state)
            .append_pair("nonce", &nonce)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(LoginStart {
            flow_id,
            authorize_url: url.into(),
        })
    }

    /// Finish a sign-in with the address OpenAI sent the browser to. A flow works once.
    pub async fn complete_login(&self, callback_url: &str) -> Result<LoginDone, String> {
        let url = url::Url::parse(callback_url.trim()).map_err(|_| {
            "That is not the address OpenAI sent you to. Paste the full address.".to_owned()
        })?;
        let param = |name: &str| {
            url.query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
        };
        let state = param("state")
            .ok_or("That is not the address OpenAI sent you to. Paste the full address.")?;
        let flow = sqlx::query(
            "DELETE FROM engine_login_flows WHERE state = $1
             RETURNING workspace_id, label, nonce, code_verifier, redirect_uri, client_id, background_consent, expires_at",
        )
        .bind(&state)
        .fetch_optional(self.pool.db())
        .await
        .map_err(internal)?
        .ok_or("This sign-in does not match. Start again.")?;
        if flow.get::<DateTime<Utc>, _>("expires_at") < Utc::now() {
            return Err("The sign-in expired. Start again.".into());
        }
        if param("error").as_deref() == Some("access_denied") {
            return Err("ChatGPT plan use was not allowed.".into());
        }
        let code = param("code").ok_or("The sign-in did not return a code. Start again.")?;
        let client_id = match flow.get::<String, _>("client_id") {
            c if c == DYNAMIC_CLIENT => {
                param("client_id").ok_or("OpenAI issued no client id. Start again.")?
            }
            c => c,
        };
        let response = self
            .token_request(&[
                ("grant_type", "authorization_code"),
                ("client_id", &client_id),
                ("code", &code),
                ("code_verifier", &flow.get::<String, _>("code_verifier")),
                ("redirect_uri", &flow.get::<String, _>("redirect_uri")),
                ("resource", RESOURCE),
            ])
            .await
            .map_err(|e| match e {
                TokenError::Rejected(code) if code == "invalid_grant" => {
                    "The sign-in failed. Start again.".to_owned()
                }
                TokenError::Rejected(code) => format!("OpenAI refused the sign-in ({code})."),
                TokenError::Transient(msg) => format!("OpenAI could not be reached: {msg}"),
            })?;
        if response
            .scope
            .as_deref()
            .is_some_and(|s| !s.split(' ').any(|p| p == NEEDED_SCOPE))
        {
            return Err(
                "This ChatGPT account cannot share plan use. Use another account or an API key."
                    .into(),
            );
        }
        let id_token = response
            .id_token
            .clone()
            .ok_or("OpenAI returned no ID token. Start again.")?;
        let claims = check_id_token(
            &id_token,
            &self.cfg.auth_base,
            &client_id,
            &flow.get::<String, _>("nonce"),
        )?;
        let email = claims["email"].as_str().map(str::to_owned);
        let label: String = flow.get("label");
        let account_id = self
            .save_account(
                flow.get("workspace_id"),
                &label,
                &client_id,
                &Tokens {
                    access_token: response.access_token,
                    refresh_token: response.refresh_token,
                    id_token,
                },
                email.as_deref(),
                expires_at(response.expires_in),
                flow.get::<bool, _>("background_consent"),
            )
            .await?;
        Ok(LoginDone {
            account_id,
            label,
            email,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn save_account(
        &self,
        workspace_id: Uuid,
        label: &str,
        client_id: &str,
        tokens: &Tokens,
        email: Option<&str>,
        expires_at: DateTime<Utc>,
        background_consent: bool,
    ) -> Result<Uuid, String> {
        let backend = self
            .pool
            .ensure_backend(workspace_id, Provider::Openai)
            .await
            .map_err(internal)?;
        let existing: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT id, mode FROM engine_accounts WHERE backend_id = $1 AND label = $2",
        )
        .bind(backend)
        .bind(label)
        .fetch_optional(self.pool.db())
        .await
        .map_err(internal)?;
        let id = match existing {
            Some((_, mode)) if mode != "plan" => {
                return Err(format!(
                    "The label {label} names an API key. Use another label."
                ));
            }
            Some((id, _)) => id,
            None => Uuid::new_v4(),
        };
        let sealed = self.pool.keyring().seal(
            id,
            serde_json::to_string(tokens).map_err(internal)?.as_bytes(),
        );
        sqlx::query(
            "INSERT INTO engine_accounts
                 (id, backend_id, mode, label, account_ref, credential_ciphertext, identity_email, expires_at, background_consent_at)
             VALUES ($1, $2, 'plan', $3, $4, $5, $6, $7, CASE WHEN $8 THEN now() END)
             ON CONFLICT (id) DO UPDATE SET
                 account_ref = excluded.account_ref, credential_ciphertext = excluded.credential_ciphertext,
                 identity_email = excluded.identity_email, expires_at = excluded.expires_at,
                 background_consent_at = excluded.background_consent_at,
                 disabled_cause = NULL, refresh_retry_at = NULL, enabled = true",
        )
        .bind(id)
        .bind(backend)
        .bind(label)
        .bind(client_id)
        .bind(sealed)
        .bind(email)
        .bind(expires_at)
        .bind(background_consent)
        .execute(self.pool.db())
        .await
        .map_err(internal)?;
        Ok(id)
    }

    async fn token_request(&self, form: &[(&str, &str)]) -> Result<TokenResponse, TokenError> {
        let resp = self
            .http
            .post(format!("{}/api/accounts/oauth/token", self.cfg.auth_base))
            .timeout(REFRESH_TIMEOUT)
            .form(form)
            .send()
            .await
            .map_err(|e| TokenError::Transient(e.without_url().to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| TokenError::Transient(e.without_url().to_string()))?;
        if status.is_success() {
            return serde_json::from_str(&body)
                .map_err(|e| TokenError::Transient(format!("bad token reply: {e}")));
        }
        if status.is_server_error() {
            return Err(TokenError::Transient(format!("OpenAI answered {status}")));
        }
        let code = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| {
                v["error"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| v["error"]["code"].as_str().map(str::to_owned))
            })
            .unwrap_or_else(|| format!("http_{}", status.as_u16()));
        Err(TokenError::Rejected(code))
    }

    /// A usable access token for a plan account. Refreshes first when it ends soon, or when
    /// `force` is set (after a 401).
    pub async fn access_token(&self, account_id: Uuid, force: bool) -> Result<Secret, EngineError> {
        let (tokens, expires, blob) = self.load(account_id).await?;
        if !force && expires > Utc::now() + Duration::seconds(EXPIRY_MARGIN_SECS) {
            return Ok(Secret::new(tokens.access_token));
        }
        Ok(Secret::new(
            self.refresh(account_id, &blob).await?.access_token,
        ))
    }

    async fn load(
        &self,
        account_id: Uuid,
    ) -> Result<(Tokens, DateTime<Utc>, Vec<u8>), EngineError> {
        let row = sqlx::query(
            "SELECT credential_ciphertext, expires_at FROM engine_accounts WHERE id = $1",
        )
        .bind(account_id)
        .fetch_optional(self.pool.db())
        .await?
        .ok_or(EngineError::NeedsLogin)?;
        let blob: Vec<u8> = row
            .get::<Option<Vec<u8>>, _>("credential_ciphertext")
            .ok_or(EngineError::NeedsLogin)?;
        let plain = self
            .pool
            .keyring()
            .open(account_id, &blob)
            .map_err(|e| EngineError::Unavailable(format!("account {account_id}: {e}")))?;
        let tokens = serde_json::from_slice(&plain)
            .map_err(|e| EngineError::Unavailable(format!("bad stored tokens: {e}")))?;
        Ok((
            tokens,
            row.get::<Option<DateTime<Utc>>, _>("expires_at")
                .unwrap_or_else(Utc::now),
            blob,
        ))
    }

    /// Refresh one account. One worker at a time (a lease row); the others wait and read the result.
    /// OpenAI rotates the refresh token, so the save compares the old blob and never overwrites a
    /// newer one. `seen` is the blob the caller read.
    async fn refresh(&self, account_id: Uuid, seen: &[u8]) -> Result<Tokens, EngineError> {
        let leased: Option<String> = sqlx::query_scalar(
            "INSERT INTO engine_account_refresh_leases (account_id, owner, expires_at)
             VALUES ($1, $2, now() + make_interval(secs => $3))
             ON CONFLICT (account_id) DO UPDATE SET owner = $2, expires_at = excluded.expires_at
                 WHERE engine_account_refresh_leases.expires_at < now()
             RETURNING owner",
        )
        .bind(account_id)
        .bind(&self.owner)
        .bind(LEASE_SECS as f64)
        .fetch_optional(self.pool.db())
        .await?;
        if leased.is_none() {
            // Another worker refreshes. Wait for its save.
            for _ in 0..40 {
                tokio::time::sleep(StdDuration::from_millis(250)).await;
                let (tokens, _, blob) = self.load(account_id).await?;
                if blob != seen {
                    return Ok(tokens);
                }
            }
            return Err(EngineError::Unavailable(
                "another worker holds the refresh lease".into(),
            ));
        }
        let result = self.refresh_leased(account_id, seen).await;
        sqlx::query(
            "DELETE FROM engine_account_refresh_leases WHERE account_id = $1 AND owner = $2",
        )
        .bind(account_id)
        .bind(&self.owner)
        .execute(self.pool.db())
        .await?;
        result
    }

    async fn refresh_leased(&self, account_id: Uuid, seen: &[u8]) -> Result<Tokens, EngineError> {
        // Read again under the lease: a worker that just finished may have saved a newer token.
        let (tokens, expires, blob) = self.load(account_id).await?;
        if blob != seen && expires > Utc::now() + Duration::seconds(EXPIRY_MARGIN_SECS) {
            return Ok(tokens);
        }
        let client_id: String =
            sqlx::query_scalar("SELECT account_ref FROM engine_accounts WHERE id = $1")
                .bind(account_id)
                .fetch_one(self.pool.db())
                .await?;
        let response = match self
            .token_request(&[
                ("grant_type", "refresh_token"),
                ("client_id", &client_id),
                ("refresh_token", &tokens.refresh_token),
                ("resource", RESOURCE),
            ])
            .await
        {
            Ok(r) => r,
            Err(TokenError::Rejected(code)) if DEAD_GRANTS.contains(&code.as_str()) => {
                self.pool.disable(account_id, &code).await?;
                return Err(EngineError::NeedsLogin);
            }
            Err(TokenError::Rejected(code)) => {
                // `invalid_client` and others: the account stays; the owner must look.
                return Err(EngineError::Unavailable(format!(
                    "OpenAI refused the refresh ({code})"
                )));
            }
            Err(TokenError::Transient(msg)) => {
                sqlx::query("UPDATE engine_accounts SET refresh_retry_at = now() + make_interval(mins => $2) WHERE id = $1")
                    .bind(account_id)
                    .bind(TRANSIENT_RETRY_MINS as f64)
                    .execute(self.pool.db())
                    .await?;
                return Err(EngineError::Unavailable(format!("refresh failed: {msg}")));
            }
        };
        let fresh = Tokens {
            id_token: response.id_token.unwrap_or(tokens.id_token),
            access_token: response.access_token,
            refresh_token: response.refresh_token,
        };
        let sealed = self.pool.keyring().seal(
            account_id,
            serde_json::to_string(&fresh)
                .expect("tokens serialize")
                .as_bytes(),
        );
        let saved = sqlx::query(
            "UPDATE engine_accounts SET credential_ciphertext = $2, expires_at = $3, refresh_retry_at = NULL
             WHERE id = $1 AND credential_ciphertext = $4",
        )
        .bind(account_id)
        .bind(sealed)
        .bind(expires_at(response.expires_in))
        .bind(&blob)
        .execute(self.pool.db())
        .await?;
        if saved.rows_affected() == 0 {
            // Lost the compare: another process saved first. Its token is the live one.
            return Ok(self.load(account_id).await?.0);
        }
        Ok(fresh)
    }

    /// Refresh every plan account whose token ends within five minutes, so an idle account keeps its
    /// sign-in. Returns the number refreshed. Run it about once a minute.
    pub async fn refresh_due(&self) -> Result<usize, EngineError> {
        let due: Vec<Uuid> = sqlx::query_scalar(
            "SELECT a.id FROM engine_accounts a JOIN engine_backends b ON b.id = a.backend_id
             WHERE b.provider = 'openai' AND a.mode = 'plan' AND a.enabled AND a.disabled_cause IS NULL
               AND a.expires_at < now() + interval '5 minutes'
               AND (a.earliest_refresh_at IS NULL OR a.earliest_refresh_at <= now())
               AND (a.refresh_retry_at IS NULL OR a.refresh_retry_at <= now())",
        )
        .fetch_all(self.pool.db())
        .await?;
        let mut refreshed = 0;
        for id in due {
            let (_, _, blob) = self.load(id).await?;
            match self.refresh(id, &blob).await {
                Ok(_) => refreshed += 1,
                Err(e) => tracing::warn!(account = %id, %e, "refresh failed"),
            }
        }
        Ok(refreshed)
    }
}

enum TokenError {
    /// OpenAI said no. The code is its `error` value.
    Rejected(String),
    /// The network or OpenAI failed. Try again later.
    Transient(String),
}

fn internal(e: impl std::fmt::Display) -> String {
    tracing::error!(%e, "sign-in internal error");
    "Internal error. See the log.".into()
}

fn expires_at(expires_in: Option<i64>) -> DateTime<Utc> {
    Utc::now() + Duration::seconds(expires_in.unwrap_or(3600))
}

fn random_token() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

/// PKCE verifier and its S256 challenge.
fn pkce() -> (String, String) {
    let verifier = random_token();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// Check the claims of an ID token and return them. No signature check: see the module note.
fn check_id_token(
    token: &str,
    issuer: &str,
    client_id: &str,
    nonce: &str,
) -> Result<Value, String> {
    let bad = || "The ID token from OpenAI is not valid. Start again.".to_owned();
    let payload = token.split('.').nth(1).ok_or_else(bad)?;
    let claims: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).map_err(|_| bad())?)
            .map_err(|_| bad())?;
    let aud_ok = match &claims["aud"] {
        Value::String(a) => a == client_id,
        Value::Array(list) => list.iter().any(|a| a == client_id),
        _ => false,
    };
    let iss_ok = claims["iss"]
        .as_str()
        .is_some_and(|i| i.trim_end_matches('/').starts_with(issuer));
    let exp_ok = claims["exp"]
        .as_i64()
        .is_some_and(|e| e > Utc::now().timestamp());
    if aud_ok && iss_ok && exp_ok && claims["nonce"].as_str() == Some(nonce) {
        Ok(claims)
    } else {
        Err(bad())
    }
}

/// The loopback listener for the sign-in callback. Each finished sign-in goes to `done`.
pub type LoginSink = mpsc::UnboundedSender<Result<LoginDone, String>>;

pub fn callback_router(siwc: Arc<Siwc>, done: LoginSink) -> axum::Router {
    async fn callback(
        State((siwc, done)): State<(Arc<Siwc>, LoginSink)>,
        uri: axum::http::Uri,
    ) -> (StatusCode, Html<String>) {
        let url = format!(
            "{}?{}",
            siwc.config().redirect_uri(),
            uri.query().unwrap_or("")
        );
        let result = siwc.complete_login(&url).await;
        let page = match &result {
            Ok(d) => (
                StatusCode::OK,
                format!(
                    "<p>Connected ChatGPT plan \"{}\". You can close this tab.</p>",
                    escape(&d.label)
                ),
            ),
            Err(e) => (StatusCode::BAD_REQUEST, format!("<p>{}</p>", escape(e))),
        };
        let _ = done.send(result);
        (page.0, Html(page.1))
    }
    axum::Router::new()
        .route("/auth/callback", get(callback))
        .with_state((siwc, done))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// OpenAI plan engine: `POST /v1/responses` with the plan token, as a stream.
pub struct OpenaiPlanEngine {
    siwc: Arc<Siwc>,
    http: reqwest::Client,
    /// The first model OpenAI lists for the sign-in. One lookup for the process.
    default_model: OnceCell<String>,
}

impl OpenaiPlanEngine {
    pub fn new(siwc: Arc<Siwc>) -> Self {
        Self {
            siwc,
            http: http::client(),
            default_model: OnceCell::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.siwc.cfg.api_base)
    }

    async fn model(&self, token: &Secret, asked: Option<&str>) -> Result<String, EngineError> {
        if let Some(m) = asked {
            return Ok(m.to_owned());
        }
        self.default_model
            .get_or_try_init(|| async {
                let resp = self
                    .http
                    .get(self.url("/v1/models"))
                    .bearer_auth(token.expose())
                    .send()
                    .await
                    .map_err(send_error)?;
                let status = resp.status().as_u16();
                let body = resp.text().await.map_err(send_error)?;
                if status >= 400 {
                    return Err(map_status(status, None, &body));
                }
                let v: Value = serde_json::from_str(&body)
                    .map_err(|e| EngineError::Unavailable(format!("bad models reply: {e}")))?;
                v["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|m| m["visibility"] == "list")
                    .and_then(|m| m["id"].as_str())
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        EngineError::Unavailable("OpenAI lists no model for this sign-in".into())
                    })
            })
            .await
            .cloned()
    }

    /// POST a streaming request. A 401 refreshes the token once and runs again.
    async fn stream(&self, account: &Account, body: &Value) -> Result<Value, EngineError> {
        let mut token = self.siwc.access_token(account.id, false).await?;
        for attempt in 0..2 {
            let resp = self
                .http
                .post(self.url("/v1/responses"))
                .bearer_auth(token.expose())
                .json(body)
                .send()
                .await
                .map_err(send_error)?;
            let status = resp.status().as_u16();
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let text = resp.text().await.map_err(send_error)?;
            if status == 401 && attempt == 0 {
                token = self.siwc.access_token(account.id, true).await?;
                continue;
            }
            if status >= 400 {
                return Err(map_status(status, retry_after.as_deref(), &text));
            }
            return completed_response(&text);
        }
        Err(EngineError::Unauthorized)
    }
}

/// The `response` of the `response.completed` event. A stream with no such event is not an answer.
fn completed_response(sse: &str) -> Result<Value, EngineError> {
    for event in sse
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok())
    {
        match event["type"].as_str() {
            Some("response.completed") => return Ok(event["response"].clone()),
            Some("response.failed") => {
                let err = &event["response"]["error"];
                let limit = err["code"]
                    .as_str()
                    .is_some_and(|c| c.contains("usage_limit") || c.contains("rate_limit"));
                return Err(if limit {
                    EngineError::RateLimited {
                        retry_at: None,
                        usage_limit: err["code"]
                            .as_str()
                            .is_some_and(|c| c.contains("usage_limit")),
                    }
                } else {
                    EngineError::Unavailable(format!("response failed: {err}"))
                });
            }
            Some("response.incomplete") => {
                return Err(EngineError::Unavailable("response incomplete".into()));
            }
            _ => {}
        }
    }
    Err(EngineError::Unavailable(
        "the stream ended with no response.completed".into(),
    ))
}

#[async_trait]
impl Engine for OpenaiPlanEngine {
    fn provider(&self) -> Provider {
        Provider::Openai
    }

    fn mode(&self) -> AccountMode {
        AccountMode::Plan
    }

    async fn run(
        &self,
        account: &Account,
        request: &EngineRequest,
    ) -> Result<EngineAnswer, EngineError> {
        let token = self.siwc.access_token(account.id, false).await?;
        let model = self.model(&token, request.model.as_deref()).await?;
        let body = json!({
            "model": model, "stream": true, "store": false,
            "instructions": "Search the web and cite your sources.",
            "include": ["web_search_call.action.sources"],
            "tools": [{"type": "web_search"}],
            "input": [{"role": "user", "content": request.prompt}],
        });
        let response = self.stream(account, &body).await?;
        let mut answer = parse_openai_response(&response, &model)?;
        // The plan pays for the call. It costs no money.
        answer.cost_cents = Some(0.0);
        Ok(answer)
    }

    async fn complete(
        &self,
        account: &Account,
        request: &CompletionRequest,
    ) -> Result<Completion, EngineError> {
        let body = json!({
            "model": request.model, "stream": true, "store": false,
            "instructions": request.system,
            "input": [{"role": "user", "content": request.user}],
        });
        let response = self.stream(account, &body).await?;
        let mut done = parse_openai_completion(&response, &request.model)?;
        done.cost_cents = Some(0.0);
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: &Value) -> String {
        format!("h.{}.s", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    #[test]
    fn pkce_challenge_is_the_sha256_of_the_verifier() {
        let (verifier, challenge) = pkce();
        assert_eq!(
            challenge,
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
        );
        assert!(
            verifier.len() >= 43,
            "RFC 7636 wants at least 43 characters"
        );
    }

    #[test]
    fn id_token_needs_the_right_audience_issuer_nonce_and_a_future_expiry() {
        let exp = Utc::now().timestamp() + 600;
        let ok = jwt(
            &json!({"iss": "https://auth.openai.com", "aud": "c1", "exp": exp, "nonce": "n1", "email": "a@b.c"}),
        );
        assert_eq!(
            check_id_token(&ok, "https://auth.openai.com", "c1", "n1").unwrap()["email"],
            "a@b.c"
        );
        assert!(check_id_token(&ok, "https://auth.openai.com", "other", "n1").is_err());
        assert!(check_id_token(&ok, "https://auth.openai.com", "c1", "wrong").is_err());
        assert!(check_id_token(&ok, "https://evil.example", "c1", "n1").is_err());
        let old =
            jwt(&json!({"iss": "https://auth.openai.com", "aud": ["c1"], "exp": 1, "nonce": "n1"}));
        assert!(check_id_token(&old, "https://auth.openai.com", "c1", "n1").is_err());
    }

    #[test]
    fn a_usage_limit_after_streaming_starts_is_a_plan_limit() {
        let sse = "data: {\"type\":\"response.created\"}\n\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\"}}}\n\n";
        assert!(matches!(
            completed_response(sse),
            Err(EngineError::RateLimited {
                usage_limit: true,
                ..
            })
        ));
        assert!(matches!(
            completed_response("data: {\"type\":\"response.created\"}\n"),
            Err(EngineError::Unavailable(_))
        ));
    }

    #[test]
    fn only_the_completed_event_gives_an_answer() {
        let sse = "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[]}}\n\n";
        assert_eq!(completed_response(sse).unwrap()["status"], "completed");
    }
}
