//! The small HTML pages the API serves itself: OAuth consent and agent-connect approval,
//! plus the helpers they share (public origin, page shell, per-IP limit, who is signing in).

use crate::{
    auth::{SESSION_COOKIE, SessionUser, password_principal, session_principal},
    state::AppState,
};
use axum::{
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

/// The origin people and agents use to reach this server, without a trailing slash.
/// `OPENJAM_PUBLIC_URL` wins. Otherwise it comes from the proxy headers, which a caller can
/// set, so a public install should set the variable.
pub fn public_base(headers: &HeaderMap) -> String {
    if let Some(url) = std::env::var("OPENJAM_PUBLIC_URL")
        .ok()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty())
    {
        return url;
    }
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let scheme = get("x-forwarded-proto").unwrap_or("http");
    let host = get("x-forwarded-host")
        .or_else(|| get("host"))
        .unwrap_or("localhost");
    format!("{scheme}://{host}")
}

/// `true` while this caller is under `max` calls in `window` for `bucket`.
/// Memory only: it resets on restart and each API process counts alone.
pub fn within_limit(bucket: &str, headers: &HeaderMap, max: usize, window: Duration) -> bool {
    static HITS: LazyLock<Mutex<HashMap<String, Vec<Instant>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    // The last entry is the one our own proxy appended; earlier ones come from the caller.
    let ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .map_or("direct", str::trim);
    let now = Instant::now();
    let mut hits = HITS.lock().unwrap_or_else(|e| e.into_inner());
    if hits.len() > 10_000 {
        hits.retain(|_, v| v.last().is_some_and(|t| now.duration_since(*t) < window));
    }
    let list = hits.entry(format!("{bucket}:{ip}")).or_default();
    list.retain(|t| now.duration_since(*t) < window);
    if list.len() >= max {
        return false;
    }
    list.push(now);
    true
}

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// A full HTML page. It cannot be framed: both pages ask a person to approve access.
pub fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{title}</title><style>\
         body{{font:16px/1.5 system-ui,sans-serif;margin:0;min-height:100vh;display:grid;place-items:center;\
         background:hsl(220 20% 97%);color:hsl(220 25% 12%)}}\
         main{{width:min(26rem,92vw);background:hsl(0 0% 100%);padding:2rem;border-radius:.75rem;\
         border:1px solid hsl(220 15% 85%)}}\
         h1{{font-size:1.25rem;margin:0 0 1rem}}\
         code{{font-size:1.1rem;letter-spacing:.08em;font-variant-numeric:tabular-nums}}\
         label{{display:block;margin:.75rem 0 .25rem;font-weight:600}}\
         input{{width:100%;box-sizing:border-box;padding:.5rem;border:1px solid hsl(220 15% 70%);border-radius:.375rem}}\
         .row{{display:flex;gap:.75rem;margin-top:1.25rem}}\
         button{{flex:1;padding:.6rem;border-radius:.375rem;border:1px solid hsl(220 15% 70%);\
         background:hsl(0 0% 100%);font:inherit;cursor:pointer}}\
         button.go{{background:hsl(221 83% 45%);border-color:hsl(221 83% 45%);color:hsl(0 0% 100%)}}\
         .err{{color:hsl(0 70% 38%)}}\
         </style></head><body><main>{body}</main></body></html>",
        title = escape(title),
    );
    let mut response = (status, html).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("frame-ancestors 'none'"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The fields a consent form adds when the person has no dashboard session.
pub fn sign_in_fields(user: Option<&SessionUser>, failed: bool) -> String {
    match user {
        Some(u) => format!("<p>Signed in as <strong>{}</strong>.</p>", escape(&u.email)),
        None => format!(
            "{}<label for=\"email\">Email</label><input id=\"email\" name=\"email\" type=\"email\" autocomplete=\"username\" required>\
             <label for=\"password\">Password</label><input id=\"password\" name=\"password\" type=\"password\" autocomplete=\"current-password\" required>",
            if failed {
                "<p class=\"err\">The email or password is wrong.</p>"
            } else {
                ""
            }
        ),
    }
}

/// Who is approving: the dashboard session if there is one, else a checked email and password.
pub async fn approver(
    state: &AppState,
    jar: &CookieJar,
    email: Option<&str>,
    password: Option<&str>,
) -> crate::Result<Option<SessionUser>> {
    if let Some(cookie) = jar.get(SESSION_COOKIE)
        && let Ok(user) = session_principal(&state.pool, cookie.value()).await
    {
        return Ok(Some(user));
    }
    match (email, password) {
        (Some(e), Some(p)) if !e.is_empty() => password_principal(&state.pool, e, p).await,
        _ => Ok(None),
    }
}
