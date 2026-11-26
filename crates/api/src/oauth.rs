//! OAuth 2.1 for agents: discovery documents, dynamic client registration, the consent page,
//! the token endpoint (code flow with PKCE, refresh rotation) and revocation.
//! Clients are public: no client secret, PKCE `S256` is required.

use crate::{
    auth::SessionUser,
    pages::{approver, escape, page, public_base, sign_in_fields, within_limit},
    state::AppState,
};
use axum::{
    Form, Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::CookieJar;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use openjam_core::auth::{
    OAUTH_ACCESS_PREFIX, OAUTH_REFRESH_PREFIX, new_prefixed_token, sha256_hex,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::time::Duration;
use subtle::ConstantTimeEq;
use uuid::Uuid;

const ACCESS_SECS: i64 = 3600;
const REFRESH_DAYS: i32 = 30;
const CODE_SECS: i64 = 600;

fn json_response(status: StatusCode, body: Value) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

fn oauth_error(status: StatusCode, code: &str, description: &str) -> Response {
    json_response(
        status,
        json!({ "error": code, "error_description": description }),
    )
}

// ---- discovery ------------------------------------------------------------------------------

/// RFC 9728: tells an MCP client which server signs people in for `/mcp`.
pub async fn protected_resource(headers: HeaderMap) -> Json<Value> {
    let base = public_base(&headers);
    Json(json!({
        "resource": format!("{base}/mcp"),
        "authorization_servers": [base],
        "bearer_methods_supported": ["header"],
    }))
}

/// RFC 8414.
pub async fn authorization_server(headers: HeaderMap) -> Json<Value> {
    let base = public_base(&headers);
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "revocation_endpoint": format!("{base}/oauth/revoke"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
    }))
}

// ---- dynamic client registration (RFC 7591) -------------------------------------------------

#[derive(Deserialize)]
pub struct Registration {
    client_name: Option<String>,
    #[serde(default)]
    redirect_uris: Vec<String>,
}

/// A redirect URI is safe when it is `https`, a loopback `http` address, or an app scheme.
fn redirect_uri_ok(uri: &str) -> bool {
    let Ok(url) = url::Url::parse(uri) else {
        return false;
    };
    if url.fragment().is_some() || uri.len() > 2000 {
        return false;
    }
    match url.scheme() {
        "https" => url.host_str().is_some(),
        "http" => is_loopback(&url),
        "javascript" | "data" | "file" | "vbscript" | "about" | "blob" | "ftp" => false,
        _ => true,
    }
}

fn is_loopback(url: &url::Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

/// Exact match, except that a loopback `http` redirect may use any port (RFC 8252).
fn redirect_matches(registered: &str, asked: &str) -> bool {
    if registered == asked {
        return true;
    }
    let (Ok(a), Ok(b)) = (url::Url::parse(registered), url::Url::parse(asked)) else {
        return false;
    };
    a.scheme() == "http"
        && b.scheme() == "http"
        && is_loopback(&a)
        && a.host_str() == b.host_str()
        && a.path() == b.path()
        && a.query() == b.query()
}

pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Registration>,
) -> Response {
    if !within_limit("oauth-register", &headers, 20, Duration::from_secs(3600)) {
        return oauth_error(
            StatusCode::TOO_MANY_REQUESTS,
            "invalid_client_metadata",
            "too many registrations; try again later",
        );
    }
    let name = body
        .client_name
        .map(|n| n.trim().chars().take(80).collect::<String>())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "MCP client".into());
    if body.redirect_uris.is_empty()
        || body.redirect_uris.len() > 10
        || !body.redirect_uris.iter().all(|u| redirect_uri_ok(u))
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "send 1 to 10 redirect_uris: https, loopback http, or an app scheme",
        );
    }
    let id: Result<Uuid, _> = sqlx::query_scalar(
        "INSERT INTO oauth_clients (name, redirect_uris) VALUES ($1, $2) RETURNING id",
    )
    .bind(&name)
    .bind(&body.redirect_uris)
    .fetch_one(&state.pool)
    .await;
    match id {
        Ok(id) => json_response(
            StatusCode::CREATED,
            json!({
                "client_id": id,
                "client_name": name,
                "redirect_uris": body.redirect_uris,
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
            }),
        ),
        Err(e) => {
            tracing::error!(error = %e, "client registration failed");
            oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "internal error",
            )
        }
    }
}

// ---- consent page ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct AuthorizeParams {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    state: Option<String>,
}

#[derive(Deserialize)]
pub struct AuthorizeForm {
    #[serde(flatten)]
    params: AuthorizeParams,
    decision: Option<String>,
    email: Option<String>,
    password: Option<String>,
}

struct Request {
    client_id: Uuid,
    client_name: String,
    redirect_uri: String,
    code_challenge: String,
    state: Option<String>,
}

fn bad_request(message: &str) -> Response {
    page(
        StatusCode::BAD_REQUEST,
        "Cannot connect",
        &format!("<h1>Cannot connect</h1><p>{}</p>", escape(message)),
    )
}

/// Check the request. An error here is shown on a page and never redirected, because the
/// redirect address is not trusted until it matches the registered client.
#[allow(clippy::result_large_err)] // the error is a page response the caller returns as-is.
async fn check(pool: &PgPool, p: &AuthorizeParams) -> Result<Request, Response> {
    if p.response_type.as_deref() != Some("code") {
        return Err(bad_request("response_type must be `code`."));
    }
    let Some(client_id) = p.client_id.as_deref().and_then(|c| c.parse::<Uuid>().ok()) else {
        return Err(bad_request("client_id is missing or unknown."));
    };
    let row: Result<Option<(String, Vec<String>)>, _> =
        sqlx::query_as("SELECT name, redirect_uris FROM oauth_clients WHERE id = $1")
            .bind(client_id)
            .fetch_optional(pool)
            .await;
    let Some((client_name, registered)) = row.ok().flatten() else {
        return Err(bad_request("client_id is missing or unknown."));
    };
    let Some(redirect_uri) = p
        .redirect_uri
        .clone()
        .filter(|r| registered.iter().any(|g| redirect_matches(g, r)))
    else {
        return Err(bad_request(
            "redirect_uri does not match the one this client registered.",
        ));
    };
    let challenge = p
        .code_challenge
        .clone()
        .filter(|c| (43..=128).contains(&c.len()));
    if p.code_challenge_method.as_deref() != Some("S256") || challenge.is_none() {
        return Err(bad_request(
            "PKCE is required: send a code_challenge with code_challenge_method=S256.",
        ));
    }
    Ok(Request {
        client_id,
        client_name,
        redirect_uri,
        code_challenge: challenge.unwrap_or_default(),
        state: p.state.clone(),
    })
}

fn hidden(name: &str, value: &str) -> String {
    format!(
        "<input type=\"hidden\" name=\"{name}\" value=\"{}\">",
        escape(value)
    )
}

fn consent_page(
    req: &Request,
    p: &AuthorizeParams,
    user: Option<&SessionUser>,
    failed: bool,
) -> Response {
    let mut fields = String::new();
    for (name, value) in [
        ("response_type", p.response_type.as_deref()),
        ("client_id", p.client_id.as_deref()),
        ("redirect_uri", p.redirect_uri.as_deref()),
        ("code_challenge", p.code_challenge.as_deref()),
        ("code_challenge_method", p.code_challenge_method.as_deref()),
        ("state", p.state.as_deref()),
    ] {
        if let Some(v) = value {
            fields.push_str(&hidden(name, v));
        }
    }
    let status = if failed {
        StatusCode::UNAUTHORIZED
    } else {
        StatusCode::OK
    };
    page(
        status,
        "Connect an app",
        &format!(
            "<h1>Connect {name} to openJam</h1>\
             <p><strong>{name}</strong> wants to read and change your workspace data. \
             It can do what an API key can do. You can revoke it later.</p>\
             <form method=\"post\" action=\"/oauth/authorize\">{fields}{sign_in}\
             <div class=\"row\"><button type=\"submit\" name=\"decision\" value=\"deny\" formnovalidate>Deny</button>\
             <button class=\"go\" type=\"submit\" name=\"decision\" value=\"allow\">Allow</button></div></form>",
            name = escape(&req.client_name),
            sign_in = sign_in_fields(user, failed),
        ),
    )
}

pub async fn authorize_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(params): Query<AuthorizeParams>,
) -> Response {
    let req = match check(&state.pool, &params).await {
        Ok(r) => r,
        Err(response) => return response,
    };
    let user = approver(&state, &jar, None, None).await.ok().flatten();
    consent_page(&req, &params, user.as_ref(), false)
}

fn redirect_with(uri: &str, pairs: &[(&str, &str)]) -> Response {
    match url::Url::parse(uri) {
        Ok(mut url) => {
            url.query_pairs_mut().extend_pairs(pairs);
            Redirect::to(url.as_str()).into_response()
        }
        Err(_) => bad_request("redirect_uri is not valid."),
    }
}

pub async fn authorize_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(form): Form<AuthorizeForm>,
) -> Response {
    let req = match check(&state.pool, &form.params).await {
        Ok(r) => r,
        Err(response) => return response,
    };
    let state_pair = req.state.as_deref().map(|s| ("state", s));
    if form.decision.as_deref() != Some("allow") {
        let mut pairs = vec![("error", "access_denied")];
        pairs.extend(state_pair);
        return redirect_with(&req.redirect_uri, &pairs);
    }
    let password_try = form.email.is_some();
    if password_try && !within_limit("oauth-login", &headers, 10, Duration::from_secs(300)) {
        return bad_request("Too many sign-in attempts. Wait a few minutes.");
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
        Err(_) => return bad_request("Something went wrong. Try again."),
    };
    let Some(user) = user else {
        return consent_page(&req, &form.params, None, true);
    };
    let code = new_prefixed_token("occ_", 40);
    let saved = sqlx::query(
        "INSERT INTO oauth_codes (code_hash, client_id, user_id, workspace_id, redirect_uri, code_challenge, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now() + make_interval(secs => $7))",
    )
    .bind(sha256_hex(&code))
    .bind(req.client_id)
    .bind(user.user_id)
    .bind(user.workspace_id)
    .bind(&req.redirect_uri)
    .bind(&req.code_challenge)
    .bind(CODE_SECS as f64)
    .execute(&state.pool)
    .await;
    if let Err(e) = saved {
        tracing::error!(error = %e, "saving an authorization code failed");
        return bad_request("Something went wrong. Try again.");
    }
    let mut pairs = vec![("code", code.as_str())];
    pairs.extend(state_pair);
    redirect_with(&req.redirect_uri, &pairs)
}

// ---- token endpoint -------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct TokenForm {
    grant_type: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
}

fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    let computed = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    computed.as_bytes().ct_eq(challenge.as_bytes()).into()
}

fn invalid_grant(description: &str) -> Response {
    oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", description)
}

/// Make a token pair for a user and return the response body.
async fn issue(
    pool: &PgPool,
    client_id: Uuid,
    user_id: Uuid,
    workspace_id: Uuid,
) -> Result<Value, sqlx::Error> {
    let access = new_prefixed_token(OAUTH_ACCESS_PREFIX, 40);
    let refresh = new_prefixed_token(OAUTH_REFRESH_PREFIX, 40);
    sqlx::query(
        "INSERT INTO oauth_tokens (client_id, user_id, workspace_id, access_hash, refresh_hash, expires_at)
         VALUES ($1, $2, $3, $4, $5, now() + make_interval(secs => $6))",
    )
    .bind(client_id)
    .bind(user_id)
    .bind(workspace_id)
    .bind(sha256_hex(&access))
    .bind(sha256_hex(&refresh))
    .bind(ACCESS_SECS as f64)
    .execute(pool)
    .await?;
    Ok(json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": ACCESS_SECS,
        "refresh_token": refresh,
    }))
}

pub async fn token(State(state): State<AppState>, Form(form): Form<TokenForm>) -> Response {
    let Some(client_id) = form
        .client_id
        .as_deref()
        .and_then(|c| c.parse::<Uuid>().ok())
    else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client",
            "client_id is missing",
        );
    };
    let outcome = match form.grant_type.as_deref() {
        Some("authorization_code") => code_grant(&state.pool, client_id, &form).await,
        Some("refresh_token") => refresh_grant(&state.pool, client_id, &form).await,
        _ => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                "use authorization_code or refresh_token",
            );
        }
    };
    match outcome {
        Ok(Ok(body)) => json_response(StatusCode::OK, body),
        Ok(Err(response)) => response,
        Err(e) => {
            tracing::error!(error = %e, "token request failed");
            oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "internal error",
            )
        }
    }
}

type Grant = Result<Result<Value, Response>, sqlx::Error>;

async fn code_grant(pool: &PgPool, client_id: Uuid, form: &TokenForm) -> Grant {
    let (Some(code), Some(verifier), Some(redirect_uri)) = (
        form.code.as_deref(),
        form.code_verifier.as_deref(),
        form.redirect_uri.as_deref(),
    ) else {
        return Ok(Err(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "code, code_verifier and redirect_uri are required",
        )));
    };
    // One use only: the row is spent here even when a later check fails.
    let row: Option<(Uuid, Uuid, Uuid, String, String)> = sqlx::query_as(
        "UPDATE oauth_codes SET used_at = now()
         WHERE code_hash = $1 AND used_at IS NULL AND expires_at > now()
         RETURNING client_id, user_id, workspace_id, redirect_uri, code_challenge",
    )
    .bind(sha256_hex(code))
    .fetch_optional(pool)
    .await?;
    let Some((row_client, user_id, workspace_id, row_redirect, challenge)) = row else {
        return Ok(Err(invalid_grant("the code is unknown, used or expired")));
    };
    if row_client != client_id || row_redirect != redirect_uri {
        return Ok(Err(invalid_grant(
            "the code was issued to another client or redirect_uri",
        )));
    }
    if !pkce_matches(verifier, &challenge) {
        return Ok(Err(invalid_grant(
            "code_verifier does not match the code_challenge",
        )));
    }
    issue(pool, client_id, user_id, workspace_id).await.map(Ok)
}

async fn refresh_grant(pool: &PgPool, client_id: Uuid, form: &TokenForm) -> Grant {
    let Some(refresh) = form.refresh_token.as_deref() else {
        return Ok(Err(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "refresh_token is required",
        )));
    };
    // Rotation: the old pair stops working the moment the new one is made.
    let row: Option<(Uuid, Uuid)> = sqlx::query_as(
        "UPDATE oauth_tokens SET revoked_at = now()
         WHERE refresh_hash = $1 AND client_id = $2 AND revoked_at IS NULL
           AND created_at > now() - make_interval(days => $3)
         RETURNING user_id, workspace_id",
    )
    .bind(sha256_hex(refresh))
    .bind(client_id)
    .bind(REFRESH_DAYS)
    .fetch_optional(pool)
    .await?;
    let Some((user_id, workspace_id)) = row else {
        return Ok(Err(invalid_grant(
            "the refresh token is unknown, used or expired",
        )));
    };
    issue(pool, client_id, user_id, workspace_id).await.map(Ok)
}

// ---- revocation (RFC 7009) ------------------------------------------------------------------

#[derive(Deserialize)]
pub struct RevokeForm {
    token: String,
}

/// Always answers 200, as the RFC asks, so a caller cannot probe for valid tokens.
pub async fn revoke(State(state): State<AppState>, Form(form): Form<RevokeForm>) -> StatusCode {
    let hash = sha256_hex(&form.token);
    if let Err(e) = sqlx::query(
        "UPDATE oauth_tokens SET revoked_at = now()
         WHERE (access_hash = $1 OR refresh_hash = $1) AND revoked_at IS NULL",
    )
    .bind(hash)
    .execute(&state.pool)
    .await
    {
        tracing::error!(error = %e, "token revocation failed");
    }
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_accepts_only_the_matching_verifier() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(pkce_matches(verifier, challenge));
        assert!(!pkce_matches("other", challenge));
    }

    #[test]
    fn redirect_rules() {
        assert!(redirect_uri_ok("https://claude.ai/api/mcp/auth_callback"));
        assert!(redirect_uri_ok("http://127.0.0.1:33418/cb"));
        assert!(redirect_uri_ok(
            "cursor://anysphere.cursor-retrieval/oauth/callback"
        ));
        assert!(!redirect_uri_ok("http://evil.example/cb"));
        assert!(!redirect_uri_ok("javascript:alert(1)"));
        assert!(!redirect_uri_ok("https://a.example/cb#frag"));
        // A loopback client may pick its port per run; nothing else may differ.
        assert!(redirect_matches(
            "http://127.0.0.1/cb",
            "http://127.0.0.1:5555/cb"
        ));
        assert!(!redirect_matches(
            "http://127.0.0.1/cb",
            "http://127.0.0.1:5555/other"
        ));
        assert!(!redirect_matches(
            "https://a.example/cb",
            "https://a.example:444/cb"
        ));
    }
}
