//! `GET /v1/events/stream`: the workspace event log as server-sent events.

use crate::dispatch::envelope;
use crate::error::{ApiErrorBody, Result};
use crate::{auth::Principal, state::AppState};
use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
};
use chrono::{DateTime, Utc};
use futures_util::stream;
use serde::Deserialize;
use serde_json::Value;
use sqlx::PgPool;
use std::{collections::VecDeque, convert::Infallible, time::Duration};
use utoipa::IntoParams;
use uuid::Uuid;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StreamQuery {
    /// Send events with an id above this one. The `Last-Event-ID` header does the same.
    /// With neither, the stream starts at the next new event.
    pub after: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    #[sqlx(rename = "type")]
    event_type: String,
    category: String,
    payload: Value,
    created_at: DateTime<Utc>,
}

struct Cursor {
    pool: PgPool,
    workspace_id: Uuid,
    last: i64,
    queue: VecDeque<Event>,
}

/// Events of the workspace as `text/event-stream`. Each event has `id`, `event` (the type)
/// and `data` (the same JSON a webhook receives). Reconnect with `Last-Event-ID` to resume.
#[utoipa::path(
    get, path = "/v1/events/stream", tag = "Events", operation_id = "stream_events",
    summary = "Stream workspace events", security(("bearerAuth" = [])), params(StreamQuery),
    responses(
        (status = 200, description = "Server-sent events", content_type = "text/event-stream", body = String),
        (status = 401, body = ApiErrorBody)
    )
)]
pub async fn stream_events(
    State(state): State<AppState>,
    principal: Principal,
    headers: HeaderMap,
    Query(q): Query<StreamQuery>,
) -> Result<Response> {
    let workspace_id = principal.workspace_id();
    let resume = q.after.or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok())
    });
    let last = match resume {
        Some(id) => id,
        None => {
            sqlx::query_scalar("SELECT COALESCE(max(id), 0) FROM events WHERE workspace_id = $1")
                .bind(workspace_id)
                .fetch_one(&state.pool)
                .await?
        }
    };
    let cursor = Cursor {
        pool: state.pool.clone(),
        workspace_id,
        last,
        queue: VecDeque::new(),
    };
    // shortcut: polls once a second; ids can commit out of order under heavy concurrent
    // writes and a slow commit may be skipped. Move to LISTEN/NOTIFY if that ever matters.
    let events = stream::unfold(cursor, |mut c| async move {
        loop {
            if let Some(event) = c.queue.pop_front() {
                return Some((Ok::<_, Infallible>(event), c));
            }
            let rows: sqlx::Result<Vec<Row>> = sqlx::query_as(
                "SELECT id, type, category, payload, created_at FROM events
                 WHERE workspace_id = $1 AND id > $2 ORDER BY id LIMIT 100",
            )
            .bind(c.workspace_id)
            .bind(c.last)
            .fetch_all(&c.pool)
            .await;
            match rows {
                Ok(rows) if !rows.is_empty() => {
                    for r in rows {
                        c.last = r.id;
                        let data = envelope(
                            r.id,
                            &r.event_type,
                            &r.category,
                            r.created_at,
                            c.workspace_id,
                            &r.payload,
                        );
                        c.queue.push_back(
                            Event::default()
                                .id(r.id.to_string())
                                .event(r.event_type)
                                .data(data.to_string()),
                        );
                    }
                }
                Ok(_) => tokio::time::sleep(Duration::from_secs(1)).await,
                Err(e) => {
                    tracing::error!(error = %e, "event stream query failed");
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
            }
        }
    });
    let mut response = Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response();
    // Tell nginx not to hold the stream back.
    response.headers_mut().insert(
        "x-accel-buffering",
        axum::http::HeaderValue::from_static("no"),
    );
    Ok(response)
}
