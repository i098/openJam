//! Agent connect: an agent with no key asks for a code, a person approves it in the browser,
//! and the agent polls until it receives an API key.

use crate::{
    auth::{Role, SessionUser},
    error::{ApiError, ApiErrorBody, Error, Result},
    pages::{approver, escape, page, public_base, sign_in_fields, within_limit},
    state::AppState,
};
use axum::{
    Form, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use openjam_core::auth::{new_api_key, new_prefixed_token, sha256_hex};
use rand::seq::IndexedRandom;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use utoipa::ToSchema;
use uuid::Uuid;

const TTL_MINUTES: i32 = 15;
const POLL_SECS: u32 = 5;
/// No 0, O, 1, I or vowels: a code read aloud or typed by hand stays unambiguous.
const CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ23456789";

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct StartConnect {
    /// Shown on the approval page, for example `Claude Code`.
    pub name: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectStarted {
    /// Open this address in a browser to approve.
    pub connect_url: String,
    /// The code the approval page shows, `XXXX-XXXX`.
    pub user_code: String,
    /// Secret for polling. Keep it in the agent.
    pub poll_token: String,
    pub expires_at: DateTime<Utc>,
    /// Wait this long between polls.
    pub interval_seconds: u32,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectStatus {
    /// `pending`, `approved`, `claimed` (the key was handed out already) or `expired`.
    pub status: &'static str,
    /// The API key. In the first `approved` reply only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<Uuid>,
}

fn new_user_code() -> String {
    let mut rng = rand::rng();
    let pick = |rng: &mut rand::rngs::ThreadRng| -> char {
        char::from(*CODE_ALPHABET.choose(rng).unwrap_or(&b'B'))
    };
    let a: String = (0..4).map(|_| pick(&mut rng)).collect();
    let b: String = (0..4).map(|_| pick(&mut rng)).collect();
    format!("{a}-{b}")
}

/// Start a connect session. Needs no credentials; limited per caller address.
#[utoipa::path(
    post, path = "/v1/agent-connect", tag = "Agent connect", operation_id = "start_agent_connect",
    summary = "Start an agent connect session", request_body = Option<StartConnect>,
    responses(
        (status = 201, body = ConnectStarted),
        (status = 429, body = ApiErrorBody)
    )
)]
pub async fn start_agent_connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<StartConnect>>,
) -> Result<(StatusCode, Json<ConnectStarted>)> {
    if !within_limit("agent-connect", &headers, 10, Duration::from_secs(60)) {
        return Err(ApiError(Error::RateLimited));
    }
    let name = body
        .and_then(|Json(b)| b.name)
        .map(|n| n.trim().chars().take(60).collect::<String>())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Agent".into());
    let poll_token = new_prefixed_token("ojp_", 40);
    for _ in 0..5 {
        let user_code = new_user_code();
        let row: Option<DateTime<Utc>> = sqlx::query_scalar(
            "INSERT INTO agent_connect_sessions (poll_token_hash, user_code, agent_name, expires_at)
             VALUES ($1, $2, $3, now() + make_interval(mins => $4))
             ON CONFLICT (user_code) DO NOTHING RETURNING expires_at",
        )
        .bind(sha256_hex(&poll_token))
        .bind(&user_code)
        .bind(&name)
        .bind(TTL_MINUTES)
        .fetch_optional(&state.pool)
        .await?;
        if let Some(expires_at) = row {
            return Ok((
                StatusCode::CREATED,
                Json(ConnectStarted {
                    connect_url: format!("{}/connect/{user_code}", public_base(&headers)),
                    user_code,
                    poll_token,
                    expires_at,
                    interval_seconds: POLL_SECS,
                }),
            ));
        }
    }
    Err(ApiError(Error::Internal("no free connect code".into())))
}

type SessionRow = (
    Uuid,
    DateTime<Utc>,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    String,
);

/// Poll a connect session. The first poll after approval returns the key, once.
#[utoipa::path(
    get, path = "/v1/agent-connect/{pollToken}", tag = "Agent connect", operation_id = "get_agent_connect",
    summary = "Poll an agent connect session",
    params(("pollToken" = String, Path, description = "The `pollToken` from the start call")),
    responses((status = 200, body = ConnectStatus), (status = 404, body = ApiErrorBody))
)]
pub async fn get_agent_connect(
    State(state): State<AppState>,
    Path(poll_token): Path<String>,
) -> Result<Json<ConnectStatus>> {
    let mut tx = state.pool.begin().await?;
    let row: Option<SessionRow> = sqlx::query_as(
        "SELECT id, expires_at, approved_by, workspace_id, api_key_id, agent_name
         FROM agent_connect_sessions WHERE poll_token_hash = $1 FOR UPDATE",
    )
    .bind(sha256_hex(&poll_token))
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, expires_at, approved_by, workspace_id, api_key_id, agent_name)) = row else {
        return Err(ApiError(Error::NotFound("connect session")));
    };
    let plain = |status| {
        Json(ConnectStatus {
            status,
            api_key: None,
            workspace_id: None,
        })
    };
    let (Some(approver), Some(workspace_id)) = (approved_by, workspace_id) else {
        let status = if expires_at < Utc::now() {
            "expired"
        } else {
            "pending"
        };
        return Ok(plain(status));
    };
    if api_key_id.is_some() {
        return Ok(plain("claimed"));
    }
    let key = new_api_key();
    let key_id: Uuid = sqlx::query_scalar(
        "INSERT INTO api_keys (workspace_id, created_by, name, prefix, hash)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(workspace_id)
    .bind(approver)
    .bind(format!("Agent: {agent_name}"))
    .bind(&key.prefix)
    .bind(&key.hash)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("UPDATE agent_connect_sessions SET api_key_id = $2 WHERE id = $1")
        .bind(id)
        .bind(key_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(ConnectStatus {
        status: "approved",
        api_key: Some(key.secret),
        workspace_id: Some(workspace_id),
    }))
}

// ---- approval page --------------------------------------------------------------------------

fn normalize(code: &str) -> String {
    code.trim().to_uppercase()
}

fn message(status: StatusCode, text: &str) -> Response {
    page(
        status,
        "Connect an agent",
        &format!("<h1>Connect an agent</h1><p>{}</p>", escape(text)),
    )
}

/// The session behind a code that is still open: not expired and not approved.
async fn open_session(state: &AppState, code: &str) -> Result<Option<(Uuid, String)>> {
    Ok(sqlx::query_as(
        "SELECT id, agent_name FROM agent_connect_sessions
         WHERE user_code = $1 AND expires_at > now() AND approved_by IS NULL",
    )
    .bind(normalize(code))
    .fetch_optional(&state.pool)
    .await?)
}

fn approval_form(code: &str, name: &str, user: Option<&SessionUser>, failed: bool) -> Response {
    page(
        if failed {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::OK
        },
        "Connect an agent",
        &format!(
            "<h1>Connect {name} to openJam</h1>\
             <p>Check that this code is the one the agent shows:</p><p><code>{code}</code></p>\
             <p>Approving gives the agent an API key for your workspace.</p>\
             <form method=\"post\">{sign_in}\
             <div class=\"row\"><button type=\"submit\" name=\"decision\" value=\"deny\" formnovalidate>Deny</button>\
             <button class=\"go\" type=\"submit\" name=\"decision\" value=\"allow\">Approve</button></div></form>",
            name = escape(name),
            code = escape(code),
            sign_in = sign_in_fields(user, failed),
        ),
    )
}

pub async fn approval_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(code): Path<String>,
) -> Response {
    match open_session(&state, &code).await {
        Ok(Some((_, name))) => {
            let user = approver(&state, &jar, None, None).await.ok().flatten();
            approval_form(&normalize(&code), &name, user.as_ref(), false)
        }
        Ok(None) => message(
            StatusCode::NOT_FOUND,
            "This code is unknown, used or expired. Ask the agent for a new one.",
        ),
        Err(_) => message(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Something went wrong. Try again.",
        ),
    }
}

#[derive(Deserialize)]
pub struct ApprovalForm {
    decision: Option<String>,
    email: Option<String>,
    password: Option<String>,
}

pub async fn approval_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(code): Path<String>,
    Form(form): Form<ApprovalForm>,
) -> Response {
    let (id, name) = match open_session(&state, &code).await {
        Ok(Some(found)) => found,
        Ok(None) => {
            return message(
                StatusCode::NOT_FOUND,
                "This code is unknown, used or expired. Ask the agent for a new one.",
            );
        }
        Err(_) => {
            return message(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong. Try again.",
            );
        }
    };
    if form.decision.as_deref() != Some("allow") {
        let _ = sqlx::query("UPDATE agent_connect_sessions SET expires_at = now() WHERE id = $1")
            .bind(id)
            .execute(&state.pool)
            .await;
        return message(StatusCode::OK, "Denied. The agent gets no key.");
    }
    if form.email.is_some()
        && !within_limit("connect-login", &headers, 10, Duration::from_secs(300))
    {
        return message(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many sign-in attempts. Wait a few minutes.",
        );
    }
    let user = match approver(
        &state,
        &jar,
        form.email.as_deref(),
        form.password.as_deref(),
    )
    .await
    {
        Ok(user) => user,
        Err(_) => {
            return message(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong. Try again.",
            );
        }
    };
    let Some(user) = user else {
        return approval_form(&normalize(&code), &name, None, true);
    };
    // Same rule as creating a key by hand: only an owner hands out keys.
    if user.role != Role::Owner {
        return message(
            StatusCode::FORBIDDEN,
            "Only a workspace owner can connect an agent. Ask an owner to approve.",
        );
    }
    let done = sqlx::query(
        "UPDATE agent_connect_sessions SET workspace_id = $2, approved_by = $3
         WHERE id = $1 AND approved_by IS NULL AND expires_at > now()",
    )
    .bind(id)
    .bind(user.workspace_id)
    .bind(user.user_id)
    .execute(&state.pool)
    .await;
    match done {
        Ok(r) if r.rows_affected() == 1 => message(
            StatusCode::OK,
            "Approved. You can close this page; the agent has its key.",
        ),
        _ => message(StatusCode::CONFLICT, "This code was used or has expired."),
    }
}
