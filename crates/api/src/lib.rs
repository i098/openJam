//! openJam HTTP API: sessions for the dashboard, `sk_...` keys for `/v1`, and one error body.

pub mod analytics;
pub mod answers;
pub mod auth;
pub mod connect;
pub mod dispatch;
pub mod error;
pub mod health;
pub mod notifications;
pub mod oauth;
pub mod pages;
pub mod routes;
pub mod state;
pub mod stream;
pub mod types;
pub mod webhooks;

pub use error::{ApiError, Result};

use crate::{
    auth::{login, logout, me, signup},
    state::AppState,
};
use axum::{
    Json, Router, middleware,
    routing::{get, post},
};
use serde_json::json;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer},
};
use tracing::Level;

/// The full API: `/healthz`, `/readyz`, `/metrics`, `/auth`, `/v1`, `/mcp`, the OAuth
/// endpoints with their discovery documents, the `/connect` page and `/openapi.json`.
pub fn router(state: AppState) -> Router {
    let (v1, openapi) = routes::build();
    let doc = serde_json::to_value(&openapi).unwrap_or_else(|_| json!({}));
    let mcp = openjam_mcp::router(
        v1.clone().with_state(state.clone()),
        &doc,
        pages::public_base,
    );
    Router::new()
        .merge(health::router())
        .route("/auth/signup", post(signup))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
        .route("/openapi.json", get(move || async move { Json(doc) }))
        .route(
            "/.well-known/oauth-protected-resource",
            get(oauth::protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth::authorization_server),
        )
        .route("/oauth/register", post(oauth::register))
        .route(
            "/oauth/authorize",
            get(oauth::authorize_page).post(oauth::authorize_submit),
        )
        .route("/oauth/token", post(oauth::token))
        .route("/oauth/revoke", post(oauth::revoke))
        .route(
            "/connect/{code}",
            get(connect::approval_page).post(connect::approval_submit),
        )
        .merge(v1)
        .with_state(state)
        .merge(mcp)
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(middleware::from_fn(health::track))
}

/// The contract as YAML, for `crates/api/contract/openapi.yaml`.
pub fn openapi_yaml() -> String {
    routes::build()
        .1
        .to_yaml()
        .unwrap_or_else(|e| panic!("openapi serialization failed: {e}"))
}
