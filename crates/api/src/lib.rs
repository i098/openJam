//! openJam HTTP API: sessions for the dashboard, `sk_...` keys for `/v1`, and one error body.

pub mod auth;
pub mod error;
pub mod health;
pub mod routes;
pub mod state;
pub mod types;

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

/// The full API: `/healthz`, `/readyz`, `/metrics`, `/auth`, `/v1`, `/openapi.json`.
pub fn router(state: AppState) -> Router {
    let (v1, openapi) = routes::build();
    let doc = serde_json::to_value(&openapi).unwrap_or_else(|_| json!({}));
    Router::new()
        .merge(health::router())
        .route("/auth/signup", post(signup))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
        .route("/openapi.json", get(move || async move { Json(doc) }))
        .merge(v1)
        .with_state(state)
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
