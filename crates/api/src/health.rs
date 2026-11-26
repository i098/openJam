//! Liveness (`/healthz`), readiness (`/readyz`) and Prometheus metrics (`/metrics`).

use crate::state::AppState;
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;
use std::{
    fmt::Write,
    sync::atomic::{AtomicU64, Ordering::Relaxed},
    time::Instant,
};

/// Requests per status class (1xx to 5xx), and the summed handling time in microseconds.
static REQUESTS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static MICROS: AtomicU64 = AtomicU64::new(0);
/// Job states in `migrations/0001_core.sql`, in output order.
const JOB_STATUSES: [&str; 4] = ["queued", "running", "done", "dead"];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
}

/// Count every request by status class and time it. Add with `middleware::from_fn`.
pub async fn track(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let res = next.run(req).await;
    let class = usize::from(res.status().as_u16() / 100).clamp(1, 5) - 1;
    REQUESTS[class].fetch_add(1, Relaxed);
    MICROS.fetch_add(
        u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX),
        Relaxed,
    );
    res
}

/// Liveness: the process answers. It never touches the database, so a database outage
/// does not make the container restart.
async fn healthz() -> Json<serde_json::Value> {
    Json(json!({"ok": true}))
}

/// Readiness: the database answers and every applied migration succeeded.
async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<serde_json::Value>) {
    let applied: Result<(i64, i64), _> = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE success), count(*) FILTER (WHERE NOT success)
         FROM _sqlx_migrations",
    )
    .fetch_one(&state.pool)
    .await;
    match applied {
        Ok((ok, 0)) if ok > 0 => (
            StatusCode::OK,
            Json(json!({"ok": true, "database": "up", "migrations": ok})),
        ),
        Ok((ok, failed)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                json!({"ok": false, "database": "up", "migrations": ok, "failed_migrations": failed}),
            ),
        ),
        Err(error) => {
            tracing::warn!(%error, "readiness check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"ok": false, "database": "down"})),
            )
        }
    }
}

/// Prometheus text format. Not proxied by the web container: scrape the API port.
async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# TYPE openjam_build_info gauge\nopenjam_build_info{{version=\"{}\"}} 1",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(out, "# TYPE openjam_http_requests_total counter");
    let mut total = 0;
    for (i, n) in REQUESTS.iter().enumerate() {
        let n = n.load(Relaxed);
        total += n;
        let _ = writeln!(
            out,
            "openjam_http_requests_total{{class=\"{}xx\"}} {n}",
            i + 1
        );
    }
    let _ = writeln!(
        out,
        "# TYPE openjam_http_request_duration_seconds summary\n\
         openjam_http_request_duration_seconds_sum {}\n\
         openjam_http_request_duration_seconds_count {total}",
        MICROS.load(Relaxed) as f64 / 1e6
    );
    let jobs: Result<Vec<(String, i64)>, _> =
        sqlx::query_as("SELECT status, count(*) FROM jobs GROUP BY status")
            .fetch_all(&state.pool)
            .await;
    let _ = writeln!(
        out,
        "# TYPE openjam_db_up gauge\nopenjam_db_up {}",
        u8::from(jobs.is_ok())
    );
    if let Ok(rows) = jobs {
        let _ = writeln!(out, "# TYPE openjam_jobs gauge");
        for status in JOB_STATUSES {
            let n = rows.iter().find(|(s, _)| s == status).map_or(0, |r| r.1);
            let _ = writeln!(out, "openjam_jobs{{status=\"{status}\"}} {n}");
        }
    }
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], out)
}
