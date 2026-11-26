//! `/v1/research*`: the state of the research runs, the latest report, and a request to run again.

use crate::analytics::find_project;
use crate::error::{ApiError, ApiErrorBody, Error, Result};
use crate::{auth::Principal, state::AppState};
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use openjam_core::research::{self, RunKind};
use serde::Serialize;
use serde_json::Value;
use sqlx::{PgPool, Row};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResearchState {
    /// `never`, `running`, `done` or `failed`.
    pub state: &'static str,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// The last run that ended.
    pub latest: Option<LatestRun>,
    /// The run in flight, if any. A run takes about ten minutes.
    pub in_flight: Option<InFlightRun>,
    /// Research again, by hand.
    pub manual: ManualResearch,
    /// Jam's own re-run. openJam starts only the first run by itself.
    pub automatic_top_up: AutomaticTopUp,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LatestRun {
    pub request_id: Uuid,
    /// `first_scan`, `backfill`, `manual` or `top_up`.
    pub kind: String,
    pub requested_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// `success` or `failed`.
    pub outcome: &'static str,
    pub failure_message: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InFlightRun {
    pub request_id: Uuid,
    pub kind: String,
    pub requested_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManualResearch {
    pub eligible: bool,
    /// Always true: openJam has no credits.
    pub free: bool,
    /// Always 0.
    pub credits: i32,
    /// `in_progress` or `first_scan_running`; null when a request would be accepted.
    pub blocked_by: Option<&'static str>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutomaticTopUp {
    pub eligible_now: bool,
    pub not_before: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResearchRequest {
    pub status: &'static str,
    pub request_id: Uuid,
    pub charged: bool,
    pub credits: i32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResearchReportResult {
    /// `null` before any run succeeded.
    pub report: Option<ResearchReport>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResearchReport {
    pub run_id: Uuid,
    pub finished_at: DateTime<Utc>,
    pub answers_read: Option<i32>,
    /// Proposals before two of the same work were merged.
    pub actions_proposed: Option<i32>,
    pub actions: Vec<ReportAction>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportAction {
    pub id: Uuid,
    pub kind: String,
    pub problem: String,
    pub status: String,
    pub title: String,
    pub why: String,
    pub details: Vec<String>,
    pub impact: String,
    pub rank: i32,
    pub target_url: Option<String>,
    pub sources: Vec<String>,
    pub questions: Value,
}

/// A scan of this project has ended with answers.
async fn has_finished_scan(pool: &PgPool, project_id: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM scan_runs WHERE project_id = $1 AND status IN ('complete', 'partial'))",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await?)
}

#[utoipa::path(
    get, path = "/v1/research", tag = "Research", operation_id = "get_research",
    summary = "Get the research state",
    description = "Research state: running or not, the latest run's timing and end, the run in flight, and whether Research again is accepted.",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ResearchState),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
    )
)]
pub async fn get_research(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<ResearchState>> {
    let project = find_project(&state, &principal, None).await?;
    let rows = sqlx::query(
        "SELECT id, kind, status, created_at, started_at, finished_at, failure_message
         FROM research_runs WHERE project_id = $1 ORDER BY created_at DESC LIMIT 20",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await?;
    let run = |r: &sqlx::postgres::PgRow| {
        (
            r.get::<Uuid, _>("id"),
            r.get::<String, _>("kind"),
            r.get::<DateTime<Utc>, _>("created_at"),
        )
    };
    let in_flight = rows
        .iter()
        .find(|r| !research::is_finished(&r.get::<String, _>("status")))
        .map(|r| {
            let (request_id, kind, requested_at) = run(r);
            InFlightRun {
                request_id,
                kind,
                requested_at,
            }
        });
    let latest = rows
        .iter()
        .find(|r| research::is_finished(&r.get::<String, _>("status")))
        .map(|r| {
            let (request_id, kind, requested_at) = run(r);
            LatestRun {
                request_id,
                kind,
                requested_at,
                started_at: r.get("started_at"),
                finished_at: r.get("finished_at"),
                outcome: if r.get::<String, _>("status") == "done" {
                    "success"
                } else {
                    "failed"
                },
                failure_message: r.get("failure_message"),
            }
        });
    let newest = rows.first();
    let state_name = match (&in_flight, &latest) {
        (Some(_), _) => "running",
        (None, Some(l)) if l.outcome == "success" => "done",
        (None, Some(_)) => "failed",
        (None, None) => "never",
    };
    let blocked_by = if in_flight.is_some() {
        Some("in_progress")
    } else if !has_finished_scan(&state.pool, project.id).await? {
        Some("first_scan_running")
    } else {
        None
    };
    Ok(Json(ResearchState {
        state: state_name,
        started_at: newest.and_then(|r| r.get("started_at")),
        finished_at: newest.and_then(|r| r.get("finished_at")),
        latest,
        in_flight,
        manual: ManualResearch {
            eligible: blocked_by.is_none(),
            free: true,
            credits: 0,
            blocked_by,
        },
        automatic_top_up: AutomaticTopUp {
            eligible_now: false,
            not_before: None,
        },
    }))
}

#[utoipa::path(
    get, path = "/v1/research/report", tag = "Research", operation_id = "get_research_report",
    summary = "Get the latest research report",
    description = "The latest successful research: answers read, and each proposed action, best first, with its why, evidence and target prompts. `report` is null before any run succeeded.",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ResearchReportResult),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
    )
)]
pub async fn get_research_report(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<ResearchReportResult>> {
    let project = find_project(&state, &principal, None).await?;
    let Some(run) = sqlx::query(
        "SELECT id, finished_at, answers_read, actions_proposed, report
         FROM research_runs WHERE project_id = $1 AND status = 'done'
         ORDER BY finished_at DESC LIMIT 1",
    )
    .bind(project.id)
    .fetch_optional(&state.pool)
    .await?
    else {
        return Ok(Json(ResearchReportResult { report: None }));
    };
    let ids: Vec<Uuid> = run
        .get::<Option<Value>, _>("report")
        .and_then(|r| serde_json::from_value(r["actionIds"].clone()).ok())
        .unwrap_or_default();
    let rows = sqlx::query(
        "SELECT id, kind, problem, status, title, why, details, impact, rank, target_url, sources, questions
         FROM actions WHERE id = ANY($1) ORDER BY rank, id",
    )
    .bind(&ids)
    .fetch_all(&state.pool)
    .await?;
    let actions = rows
        .iter()
        .map(|r| ReportAction {
            id: r.get("id"),
            kind: r.get("kind"),
            problem: r.get("problem"),
            status: r.get("status"),
            title: r.get("title"),
            why: r.get::<Option<String>, _>("why").unwrap_or_default(),
            details: r
                .get::<Option<Value>, _>("details")
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default(),
            impact: r.get::<Option<String>, _>("impact").unwrap_or_default(),
            rank: r.get("rank"),
            target_url: r.get("target_url"),
            sources: r.get("sources"),
            questions: r.get("questions"),
        })
        .collect();
    Ok(Json(ResearchReportResult {
        report: Some(ResearchReport {
            run_id: run.get("id"),
            finished_at: run.get("finished_at"),
            answers_read: Some(run.get("answers_read")),
            actions_proposed: run.get("actions_proposed"),
            actions,
        }),
    }))
}

#[utoipa::path(
    post, path = "/v1/research-requests", tag = "Research", operation_id = "create_research_request",
    summary = "Research again",
    description = "Ask Jam to research the active project again and rewrite the Plan. Takes about ten minutes; poll `GET /v1/research`. 409 while a run is in flight or the first scan has not finished.",
    security(("bearerAuth" = [])),
    responses(
        (status = 202, body = ResearchRequest),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody),
    )
)]
pub async fn create_research_request(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<(StatusCode, Json<ResearchRequest>)> {
    let project = find_project(&state, &principal, None).await?;
    if !has_finished_scan(&state.pool, project.id).await? {
        return Err(ApiError(Error::Conflict(
            "the first scan has not finished; research reads its answers".into(),
        )));
    }
    let request_id = research::request_run(&state.pool, project.id, RunKind::Manual).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(ResearchRequest {
            status: "accepted",
            request_id,
            charged: false,
            credits: 0,
        }),
    ))
}
