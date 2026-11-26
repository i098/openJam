//! `/v1/visibility-runs`: start a scan and read it back.

use crate::analytics::find_project;
use crate::auth::Principal;
use crate::error::{ApiError, ApiErrorBody, Error, Result};
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use openjam_core::tracking;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityRun {
    pub run_id: Uuid,
    pub project_id: Uuid,
    pub prompt_count: i32,
    pub platform_count: i32,
    /// A rough estimate from the number of calls, 15 seconds each.
    pub expected_minutes: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityRunSummary {
    pub run_id: Uuid,
    pub project_id: Uuid,
    /// `queued`, `running`, `complete`, `partial`, `failed` or `cap_reached`.
    pub status: String,
    pub prompt_count: i32,
    pub platform_count: i32,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct VisibilityRunList {
    pub runs: Vec<VisibilityRunSummary>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StartRun {
    /// The project; the newest one when omitted.
    pub project_id: Option<Uuid>,
}

/// Start a scan: every tracked, active prompt on every platform of the project.
#[utoipa::path(
    post, path = "/v1/visibility-runs", tag = "Visibility runs", operation_id = "create_visibility_run",
    summary = "Start a scan", security(("bearerAuth" = [])), request_body = StartRun,
    responses(
        (status = 202, body = VisibilityRun),
        (status = 401, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody),
        (status = 429, body = ApiErrorBody)
    )
)]
pub async fn create_visibility_run(
    State(state): State<AppState>,
    principal: Principal,
    Json(body): Json<StartRun>,
) -> Result<(StatusCode, Json<VisibilityRun>)> {
    let project = find_project(&state, &principal, body.project_id).await?;
    let run_id = tracking::start_scan(&state.pool, project.id, state.config.scan_cap_per_day)
        .await?
        .ok_or(Error::Conflict(
            "add at least one prompt and one platform before a scan".into(),
        ))?;
    type Row = (i32, i32);
    let (prompt_count, platform_count): Row =
        sqlx::query_as("SELECT prompt_count, platform_count FROM scan_runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&state.pool)
            .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(VisibilityRun {
            run_id,
            project_id: project.id,
            prompt_count,
            platform_count,
            expected_minutes: (prompt_count as i64 * platform_count as i64 * 15 + 59) / 60,
        }),
    ))
}

#[utoipa::path(
    get, path = "/v1/visibility-runs", tag = "Visibility runs", operation_id = "list_visibility_runs",
    summary = "List scans", security(("bearerAuth" = [])),
    responses((status = 200, body = VisibilityRunList), (status = 401, body = ApiErrorBody))
)]
pub async fn list_visibility_runs(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<VisibilityRunList>> {
    let runs: Vec<VisibilityRunSummary> = sqlx::query_as(
        "SELECT id AS run_id, project_id, status, prompt_count, platform_count,
                started_at, finished_at, created_at
         FROM scan_runs WHERE project_id IN
             (SELECT id FROM projects WHERE workspace_id = $1 AND archived_at IS NULL)
         ORDER BY created_at DESC LIMIT 20",
    )
    .bind(principal.workspace_id())
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(VisibilityRunList { runs }))
}

#[utoipa::path(
    get, path = "/v1/visibility-runs/{runId}", tag = "Visibility runs", operation_id = "get_visibility_run",
    summary = "Get one scan", security(("bearerAuth" = [])),
    params(("runId" = Uuid, Path, description = "The scan run")),
    responses(
        (status = 200, body = VisibilityRunSummary),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn get_visibility_run(
    State(state): State<AppState>,
    principal: Principal,
    Path(run_id): Path<Uuid>,
) -> Result<Json<VisibilityRunSummary>> {
    let run: Option<VisibilityRunSummary> = sqlx::query_as(
        "SELECT id AS run_id, project_id, status, prompt_count, platform_count,
                started_at, finished_at, created_at
         FROM scan_runs WHERE id = $1 AND project_id IN
             (SELECT id FROM projects WHERE workspace_id = $2)",
    )
    .bind(run_id)
    .bind(principal.workspace_id())
    .fetch_optional(&state.pool)
    .await?;
    run.map(Json)
        .ok_or_else(|| ApiError(Error::NotFound("visibility run")))
}
