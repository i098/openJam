//! The workspace event log. Any slice calls [`emit`]; webhooks, the inbox and the SSE stream read it.

use crate::{Error, Result};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// Notification categories, the same set as the source product.
pub const CATEGORIES: [&str; 5] = [
    "action_required",
    "error",
    "status_update",
    "insight",
    "completion",
];

/// Record one event and return its id. `event_type` is dotted, for example `scan.complete`.
pub async fn emit(
    pool: &PgPool,
    workspace_id: Uuid,
    event_type: &str,
    category: &str,
    payload: &Value,
) -> Result<i64> {
    if !CATEGORIES.contains(&category) {
        return Err(Error::InvalidRequest(format!(
            "unknown event category {category}"
        )));
    }
    Ok(sqlx::query_scalar(
        "INSERT INTO events (workspace_id, type, category, payload) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(workspace_id)
    .bind(event_type)
    .bind(category)
    .bind(payload)
    .fetch_one(pool)
    .await?)
}
