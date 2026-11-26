//! Session cookies for the dashboard, `sk_...` bearer keys for the API, and the auth routes.

use crate::error::{ApiError, Result};
use crate::{state::AppState, types::*};
use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::header,
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use openjam_core::{
    Error,
    auth::{API_KEY_PREFIX, hash_password, new_session_token, sha256_hex, verify_password},
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

pub const SESSION_COOKIE: &str = "openjam_session";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Owner,
    Member,
}

pub struct SessionUser {
    pub user_id: Uuid,
    pub workspace_id: Uuid,
    pub role: Role,
    pub email: String,
    pub name: Option<String>,
}

pub struct KeyUser {
    pub key_id: Uuid,
    pub workspace_id: Uuid,
    pub user_id: Uuid,
    pub email: String,
    pub name: Option<String>,
}

/// Who made the request: a dashboard session or an API key.
pub enum Principal {
    Session(SessionUser),
    Key(KeyUser),
}

impl Principal {
    pub fn workspace_id(&self) -> Uuid {
        match self {
            Self::Session(s) => s.workspace_id,
            Self::Key(k) => k.workspace_id,
        }
    }

    pub fn user_id(&self) -> Uuid {
        match self {
            Self::Session(s) => s.user_id,
            Self::Key(k) => k.user_id,
        }
    }

    pub async fn me(&self, pool: &PgPool) -> openjam_core::Result<Me> {
        let (id, email, name, workspace_id) = match self {
            Self::Session(s) => (s.user_id, s.email.clone(), s.name.clone(), s.workspace_id),
            Self::Key(k) => (k.user_id, k.email.clone(), k.name.clone(), k.workspace_id),
        };
        let project: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT id, domain FROM projects WHERE workspace_id = $1 AND archived_at IS NULL
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(workspace_id)
        .fetch_optional(pool)
        .await?;
        Ok(Me {
            user: MeUser { id, name, email },
            workspace_id,
            project: project.map(|(id, domain)| MeProject { id, domain }),
        })
    }
}

impl FromRequestParts<AppState> for Principal {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &AppState,
    ) -> Result<Self> {
        if let Some(value) = parts.headers.get(header::AUTHORIZATION) {
            let token = value
                .to_str()
                .ok()
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(str::trim);
            let Some(token) = token.filter(|t| t.starts_with(API_KEY_PREFIX)) else {
                return Err(ApiError(Error::Unauthorized));
            };
            return key_principal(&state.pool, token).await.map(Self::Key);
        }
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar.get(SESSION_COOKIE).map(|c| c.value().to_string());
        let Some(token) = token else {
            return Err(ApiError(Error::Unauthorized));
        };
        session_principal(&state.pool, &token)
            .await
            .map(Self::Session)
    }
}

async fn session_principal(pool: &PgPool, token: &str) -> Result<SessionUser> {
    let row: Option<(Uuid, Uuid, String, String, Option<String>)> = sqlx::query_as(
        "SELECT u.id, m.workspace_id, m.role, u.email, u.name
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         JOIN memberships m ON m.user_id = u.id
         WHERE s.token_hash = $1 AND s.expires_at > now()
         ORDER BY m.created_at
         LIMIT 1",
    )
    .bind(sha256_hex(token))
    .fetch_optional(pool)
    .await?;
    let Some((user_id, workspace_id, role, email, name)) = row else {
        return Err(ApiError(Error::Unauthorized));
    };
    Ok(SessionUser {
        user_id,
        workspace_id,
        role: role_from(&role),
        email,
        name,
    })
}

async fn key_principal(pool: &PgPool, token: &str) -> Result<KeyUser> {
    let row: Option<(Uuid, Uuid, Uuid, String, Option<String>)> = sqlx::query_as(
        "UPDATE api_keys k SET last_used_at = now()
         FROM users u
         WHERE k.hash = $1 AND k.revoked_at IS NULL AND u.id = k.created_by
         RETURNING k.id, k.workspace_id, u.id, u.email, u.name",
    )
    .bind(sha256_hex(token))
    .fetch_optional(pool)
    .await?;
    let Some((key_id, workspace_id, user_id, email, name)) = row else {
        return Err(ApiError(Error::Unauthorized));
    };
    Ok(KeyUser {
        key_id,
        workspace_id,
        user_id,
        email,
        name,
    })
}

fn role_from(role: &str) -> Role {
    if role == "owner" {
        Role::Owner
    } else {
        Role::Member
    }
}

// ---- auth routes ---------------------------------------------------------

#[derive(Deserialize, ToSchema)]
pub struct Signup {
    pub email: String,
    pub password: String,
    pub name: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct Login {
    pub email: String,
    pub password: String,
}

#[derive(Serialize, ToSchema)]
pub struct SessionInfo {
    pub user: MeUser,
    pub workspace_id: Uuid,
    pub role: String,
}

fn session_cookie(config: &openjam_core::config::Config, token: &str) -> Cookie<'static> {
    let mut cookie = Cookie::new(SESSION_COOKIE, token.to_string());
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_secure(config.cookie_secure);
    cookie.set_path("/");
    cookie.set_max_age(cookie::time::Duration::days(config.session_days));
    cookie
}

async fn start_session(
    pool: &PgPool,
    config: &openjam_core::config::Config,
    user_id: Uuid,
    email: String,
    name: Option<String>,
) -> Result<(Cookie<'static>, SessionInfo)> {
    let token = new_session_token();
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, now() + make_interval(days => $3))")
        .bind(user_id)
        .bind(sha256_hex(&token))
        .bind(config.session_days as i32)
        .execute(pool)
        .await?;
    let (workspace_id, role): (Uuid, String) = sqlx::query_as(
        "SELECT workspace_id, role FROM memberships WHERE user_id = $1 ORDER BY created_at LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok((
        session_cookie(config, &token),
        SessionInfo {
            user: MeUser {
                id: user_id,
                name,
                email,
            },
            workspace_id,
            role,
        },
    ))
}

pub async fn signup(State(state): State<AppState>, Json(body): Json<Signup>) -> Result<Response> {
    let email = body.email.trim().to_lowercase();
    if !email.contains('@') || email.len() > 254 {
        return Err(ApiError::bad("email is not valid"));
    }
    if body.password.len() < 8 {
        return Err(ApiError::bad("password must be at least 8 characters"));
    }
    if !state.config.allow_signup {
        return Err(ApiError(Error::Forbidden));
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE lower(email) = $1)")
            .bind(&email)
            .fetch_one(&state.pool)
            .await?;
    if exists {
        return Err(ApiError(Error::Conflict(
            "an account with this email exists; sign in".into(),
        )));
    }
    let hash = hash_password(&body.password)?;
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, name, password_hash) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&email)
    .bind(body.name.filter(|n| !n.trim().is_empty()))
    .bind(hash)
    .fetch_one(&state.pool)
    .await?;
    // The first user of a new install gets their own workspace.
    let workspace_id: Uuid =
        sqlx::query_scalar("INSERT INTO workspaces (name) VALUES ($1) RETURNING id")
            .bind(&email)
            .fetch_one(&state.pool)
            .await?;
    sqlx::query("INSERT INTO memberships (workspace_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(workspace_id)
        .bind(user_id)
        .execute(&state.pool)
        .await?;
    let (cookie, info) = start_session(&state.pool, &state.config, user_id, email, None).await?;
    Ok((CookieJar::new().add(cookie), Json(info)).into_response())
}

pub async fn login(State(state): State<AppState>, Json(body): Json<Login>) -> Result<Response> {
    let email = body.email.trim().to_lowercase();
    let row: Option<(Uuid, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT id, password_hash, name FROM users WHERE lower(email) = $1")
            .bind(&email)
            .fetch_optional(&state.pool)
            .await?;
    let Some((user_id, hash, name)) = row else {
        return Err(ApiError(Error::Unauthorized));
    };
    let Some(hash) = hash else {
        return Err(ApiError(Error::Unauthorized));
    };
    if !verify_password(&body.password, &hash) {
        return Err(ApiError(Error::Unauthorized));
    }
    let (cookie, info) = start_session(&state.pool, &state.config, user_id, email, name).await?;
    Ok((CookieJar::new().add(cookie), Json(info)).into_response())
}

pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> Result<CookieJar> {
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(sha256_hex(cookie.value()))
            .execute(&state.pool)
            .await?;
    }
    Ok(jar.remove(Cookie::new(SESSION_COOKIE, "")))
}

pub async fn me(State(state): State<AppState>, principal: Principal) -> Result<Json<Me>> {
    Ok(Json(principal.me(&state.pool).await?))
}
