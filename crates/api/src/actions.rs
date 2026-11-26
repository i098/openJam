//! `/v1/actions*`: the Plan. Every action has a status that comes from its history; this module
//! only reports events. An agent without a key reports through the secret token in its prompt.

use crate::analytics::{Q, find_project, types::PageInfo};
use crate::error::{ApiError, ApiErrorBody, Error, Result};
use crate::opportunities::start_plan;
use crate::{auth::Principal, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use openjam_core::actions::{self, Event, KINDS, NewEvent, Status};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row, postgres::PgRow};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

const DEFAULT_LIMIT: i64 = 100;
const STATUSES: [&str; 8] = [
    "suggested",
    "in_progress",
    "sent",
    "done",
    "verified",
    "unverified",
    "dismissed",
    "superseded",
];
const MAX_URL: usize = 2000;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub id: Uuid,
    /// `page_edit`, `new_page`, `outreach`, `technical` or `community`.
    pub kind: String,
    /// The gap the action closes.
    pub problem: String,
    pub title: String,
    pub why: String,
    /// `high`, `medium` or `low`.
    pub impact: String,
    /// Priority within the Plan; 1 is first.
    pub rank: i32,
    pub target_url: Option<String>,
    pub status: String,
    /// The article opportunity behind the action, once Jam has one.
    pub opportunity_id: Option<Uuid>,
    /// The content written for that opportunity, once there is any.
    pub content_id: Option<Uuid>,
    pub last_seen_at: DateTime<Utc>,
    /// Whether `POST /v1/actions/{id}/executions` would have Jam do this now.
    pub executable_by_jam: bool,
    /// Why not, when `executable_by_jam` is false: `closed`, `already_drafted`, `not_jam_work`,
    /// `needs_github`, `no_page`.
    pub not_executable_reason: Option<&'static str>,
    /// What to do with the action next.
    pub next_step: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Actions {
    pub actions: Vec<Action>,
    pub counts_by_status: BTreeMap<String, i64>,
    pub page_info: PageInfo,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActionDetail {
    #[serde(flatten)]
    pub action: Action,
    pub kind_label: &'static str,
    /// `research`, `planner` or `user`.
    pub source: String,
    /// The evidence and specifics research found.
    pub details: Vec<String>,
    /// Pages research read for this action.
    pub sources: Vec<String>,
    /// The tracked prompts this action should move: `{promptId, text}`.
    pub questions: Value,
    /// Everything that happened to the action, oldest first.
    pub history: Vec<HistoryEvent>,
    /// The latest check of a done claim: `{passed, evidence, url, at}`.
    pub check: Option<Value>,
    /// Instructions for your agent, ending with how to report the work.
    pub agent_prompt: String,
    /// Jam's prepared code change. Always null in this build.
    pub change: Option<Value>,
    pub first_seen_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event: String,
    pub actor: String,
    pub at: DateTime<Utc>,
    pub url: Option<String>,
    pub evidence: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// One or more statuses, comma separated.
    pub status: Option<String>,
    /// One or more kinds, comma separated.
    pub kind: Option<String>,
    /// Also list actions a later research run no longer proposes.
    pub include_superseded: Option<bool>,
    /// Items per page, 1 to 100. The default is 100.
    pub limit: Option<i64>,
    /// `pageInfo.nextCursor` of the page before.
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ActionCompletion {
    /// Where the finished work lives.
    pub url: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ActionCheck {
    /// The page to check, when it moved. Defaults to the last claim.
    pub url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ActionTransitionResult {
    pub status: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ActionExecutionRequest {
    /// `one_shot` (default): Jam drafts the work. `agent` is not available in this build.
    pub mode: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActionExecution {
    pub status: &'static str,
    /// `draft`: Jam is writing the article; follow `opportunity_id`.
    pub work: &'static str,
    pub opportunity_id: Option<Uuid>,
    pub change_id: Option<Uuid>,
    pub run_id: Option<Uuid>,
    pub chat_url: Option<String>,
}

const COLUMNS: &str = "a.id, a.kind, a.problem, a.title, a.why, a.impact, a.rank, a.target_url,
    a.status, a.opportunity_id, a.last_seen_at, a.first_seen_at, a.source, a.details, a.sources,
    a.questions, a.check_result, a.completion_token,
    (SELECT c.id FROM contents c WHERE c.opportunity_id = a.opportunity_id ORDER BY c.created_at DESC LIMIT 1) AS content_id";

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "new_page" => "New page",
        "page_edit" => "Page edit",
        "outreach" => "Outreach",
        "technical" => "Technical fix",
        _ => "Community",
    }
}

/// Whether Jam could do the action now, and if not, why.
fn executable(
    kind: &str,
    status: &str,
    opportunity: Option<Uuid>,
    content: Option<Uuid>,
) -> (bool, Option<&'static str>) {
    if matches!(status, "dismissed" | "superseded" | "verified") {
        return (false, Some("closed"));
    }
    if content.is_some() {
        return (false, Some("already_drafted"));
    }
    if matches!(kind, "outreach" | "community") {
        return (false, Some("not_jam_work"));
    }
    match (opportunity, kind) {
        (Some(_), _) => (true, None),
        (None, "technical" | "page_edit") => (false, Some("needs_github")),
        (None, _) => (false, Some("no_page")),
    }
}

fn next_step(id: Uuid, can: bool, reason: Option<&str>, status: &str) -> String {
    match (can, reason, status) {
        (true, ..) => format!(
            "POST /v1/actions/{id}/executions to have Jam write it, or do the work from agentPrompt and POST /v1/actions/{id}/completion."
        ),
        (_, Some("closed"), _) => "Nothing to do: the action is closed.".into(),
        (_, _, "done" | "sent") => {
            format!("Wait for the check, or POST /v1/actions/{id}/checks to check again.")
        }
        (_, Some("already_drafted"), _) => "Review the drafted content under /v1/content.".into(),
        _ => format!("Do the work from agentPrompt, then POST /v1/actions/{id}/completion."),
    }
}

fn action(r: &PgRow) -> Action {
    let id: Uuid = r.get("id");
    let (kind, status): (String, String) = (r.get("kind"), r.get("status"));
    let (opportunity_id, content_id): (Option<Uuid>, Option<Uuid>) =
        (r.get("opportunity_id"), r.get("content_id"));
    let (can, reason) = executable(&kind, &status, opportunity_id, content_id);
    Action {
        id,
        next_step: next_step(id, can, reason, &status),
        executable_by_jam: can,
        not_executable_reason: reason,
        kind,
        problem: r.get("problem"),
        title: r.get("title"),
        why: r.get::<Option<String>, _>("why").unwrap_or_default(),
        impact: r.get::<Option<String>, _>("impact").unwrap_or_default(),
        rank: r.get("rank"),
        target_url: r.get("target_url"),
        status,
        opportunity_id,
        content_id,
        last_seen_at: r.get("last_seen_at"),
    }
}

fn parse_list(raw: Option<&str>, allowed: &[&str], name: &str) -> Result<Option<Vec<String>>> {
    let Some(raw) = raw else { return Ok(None) };
    let values: Vec<String> = raw.split(',').map(|s| s.trim().to_owned()).collect();
    match values.iter().find(|v| !allowed.contains(&v.as_str())) {
        Some(bad) => Err(ApiError::bad(format!(
            "{name} {bad} is not one of {}",
            allowed.join(", ")
        ))),
        None => Ok(Some(values)),
    }
}

fn parse_cursor(cursor: &str) -> Result<(i32, Uuid)> {
    cursor
        .split_once('_')
        .and_then(|(rank, id)| Some((rank.parse().ok()?, id.parse().ok()?)))
        .ok_or_else(|| ApiError::bad("cursor is not valid"))
}

/// A URL a person or an agent reports. Only http and https; the worker reads it later.
fn check_url(url: Option<&str>) -> Result<Option<&str>> {
    let Some(url) = url.map(str::trim).filter(|u| !u.is_empty()) else {
        return Ok(None);
    };
    if url.len() > MAX_URL || !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(ApiError::bad("url must be an http or https address"));
    }
    Ok(Some(url))
}

async fn load(state: &AppState, principal: &Principal, id: Uuid) -> Result<PgRow> {
    sqlx::query(&format!(
        "SELECT {COLUMNS} FROM actions a JOIN projects p ON p.id = a.project_id
         WHERE a.id = $1 AND p.workspace_id = $2"
    ))
    .bind(id)
    .bind(principal.workspace_id())
    .fetch_optional(&state.pool)
    .await?
    .ok_or(ApiError(Error::NotFound("action")))
}

#[utoipa::path(
    get, path = "/v1/actions", tag = "Actions", operation_id = "list_actions",
    summary = "List the Plan",
    description = "The project's Plan, highest priority first, with the count in each status. Superseded actions are left out unless `includeSuperseded=true` or `status` names them. Paged: `limit` (default 100, at most 100) and `cursor`; `countsByStatus` covers every page.",
    security(("bearerAuth" = [])),
    params(ListQuery),
    responses(
        (status = 200, body = Actions),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
    )
)]
pub async fn list_actions(
    State(state): State<AppState>,
    principal: Principal,
    Q(q): Q<ListQuery>,
) -> Result<Json<Actions>> {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=100).contains(&limit) {
        return Err(ApiError::bad("limit must be 1 to 100"));
    }
    let statuses = parse_list(q.status.as_deref(), &STATUSES, "status")?;
    let kinds = parse_list(q.kind.as_deref(), &KINDS, "kind")?;
    let cursor = q.cursor.as_deref().map(parse_cursor).transpose()?;
    let project = find_project(&state, &principal, None).await?;
    let filter = "a.project_id = $1
        AND ($2::text[] IS NULL OR a.status = ANY($2))
        AND ($3::text[] IS NULL OR a.kind = ANY($3))
        AND (a.status <> 'superseded' OR $4 OR COALESCE('superseded' = ANY($2), false))";
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM actions a WHERE {filter}"))
        .bind(project.id)
        .bind(&statuses)
        .bind(&kinds)
        .bind(q.include_superseded.unwrap_or(false))
        .fetch_one(&state.pool)
        .await?;
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM actions a WHERE {filter}
           AND ($5::int IS NULL OR (a.rank, a.id) > ($5, $6::uuid))
         ORDER BY a.rank, a.id LIMIT $7"
    ))
    .bind(project.id)
    .bind(&statuses)
    .bind(&kinds)
    .bind(q.include_superseded.unwrap_or(false))
    .bind(cursor.map(|c| c.0))
    .bind(cursor.map(|c| c.1))
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;
    let has_next = rows.len() as i64 > limit;
    let rows = &rows[..rows.len().min(limit as usize)];
    let next_cursor = has_next
        .then(|| rows.last())
        .flatten()
        .map(|r| format!("{}_{}", r.get::<i32, _>("rank"), r.get::<Uuid, _>("id")));
    let mut counts: BTreeMap<String, i64> = STATUSES.iter().map(|s| ((*s).to_owned(), 0)).collect();
    let tallies: Vec<(String, i64)> = sqlx::query_as(
        "SELECT status, count(*) FROM actions WHERE project_id = $1 GROUP BY status",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await?;
    counts.extend(tallies);
    Ok(Json(Actions {
        actions: rows.iter().map(action).collect(),
        counts_by_status: counts,
        page_info: PageInfo {
            limit,
            has_next_page: has_next,
            next_cursor,
            total_count: total as u64,
        },
    }))
}

#[utoipa::path(
    get, path = "/v1/actions/{actionId}", tag = "Actions", operation_id = "get_action",
    summary = "Get an action",
    description = "Everything to do one action: evidence (`details`, `sources`), prompts to move (`questions`), target page, history, latest check, and `agentPrompt` (instructions for your agent, ending with how to report the work).",
    security(("bearerAuth" = [])),
    params(("actionId" = Uuid, Path, description = "The action id")),
    responses(
        (status = 200, body = ActionDetail),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
    )
)]
pub async fn get_action(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<ActionDetail>> {
    let r = load(&state, &principal, id).await?;
    let history = sqlx::query(
        "SELECT id, type, actor, at, url, evidence, note FROM action_events
         WHERE action_id = $1 ORDER BY id",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?
    .iter()
    .map(|e| HistoryEvent {
        id: e.get::<i64, _>("id").to_string(),
        event: e.get("type"),
        actor: e.get("actor"),
        at: e.get("at"),
        url: e.get("url"),
        evidence: e.get("evidence"),
        note: e.get("note"),
    })
    .collect();
    let details: Vec<String> = r
        .get::<Option<Value>, _>("details")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let base = action(&r);
    let prompt = actions::agent_prompt(
        &base.kind,
        &base.title,
        &base.why,
        base.target_url.as_deref(),
        &details,
        &r.get::<Option<String>, _>("completion_token")
            .unwrap_or_default(),
    )
    .replace("{id}", &id.to_string());
    Ok(Json(ActionDetail {
        kind_label: kind_label(&base.kind),
        source: r.get("source"),
        details,
        sources: r.get("sources"),
        questions: r.get("questions"),
        history,
        check: r.get("check_result"),
        agent_prompt: prompt,
        change: None,
        first_seen_at: r.get("first_seen_at"),
        action: base,
    }))
}

async fn transition(
    state: &AppState,
    principal: &Principal,
    id: Uuid,
    event: Event,
) -> Result<Json<ActionTransitionResult>> {
    let status = actions::record(
        &state.pool,
        principal.workspace_id(),
        id,
        &NewEvent::new(event, "user"),
    )
    .await?;
    Ok(Json(ActionTransitionResult {
        status: status.as_str().to_owned(),
    }))
}

#[utoipa::path(
    post, path = "/v1/actions/{actionId}/start", tag = "Actions", operation_id = "start_action",
    summary = "Start an action",
    description = "Record that work on the action has begun (status `in_progress`). To take a started action off the Plan, dismiss it. Safe to repeat.",
    security(("bearerAuth" = [])),
    params(("actionId" = Uuid, Path, description = "The action id")),
    responses(
        (status = 200, body = ActionTransitionResult),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody),
    )
)]
pub async fn start_action(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<ActionTransitionResult>> {
    transition(&state, &principal, id, Event::Started).await
}

#[utoipa::path(
    post, path = "/v1/actions/{actionId}/sent", tag = "Actions", operation_id = "report_action_sent",
    summary = "Report an outreach pitch sent",
    description = "Outreach only: the pitch went out; the third party has not acted yet (status `sent`). 422 for any other kind. Safe to repeat.",
    security(("bearerAuth" = [])),
    params(("actionId" = Uuid, Path, description = "The action id")),
    responses(
        (status = 200, body = ActionTransitionResult),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody),
        (status = 422, body = ApiErrorBody),
    )
)]
pub async fn report_action_sent(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<ActionTransitionResult>> {
    if load(&state, &principal, id).await?.get::<String, _>("kind") != "outreach" {
        return Err(ApiError(Error::Unprocessable(
            "only an outreach action can be sent".into(),
        )));
    }
    transition(&state, &principal, id, Event::Sent).await
}

#[utoipa::path(
    post, path = "/v1/actions/{actionId}/dismissal", tag = "Actions", operation_id = "dismiss_action",
    summary = "Dismiss an action",
    description = "Take an open action off the Plan (status `dismissed`). `DELETE` restores it. Safe to repeat.",
    security(("bearerAuth" = [])),
    params(("actionId" = Uuid, Path, description = "The action id")),
    responses(
        (status = 200, body = ActionTransitionResult),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody),
    )
)]
pub async fn dismiss_action(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<ActionTransitionResult>> {
    transition(&state, &principal, id, Event::Dismissed).await
}

#[utoipa::path(
    delete, path = "/v1/actions/{actionId}/dismissal", tag = "Actions", operation_id = "restore_action",
    summary = "Restore a dismissed action",
    description = "Put a dismissed action back on the Plan, as it was before the dismissal. Safe to repeat on a restored action only through `409`: only a dismissed action can be restored.",
    security(("bearerAuth" = [])),
    params(("actionId" = Uuid, Path, description = "The action id")),
    responses(
        (status = 200, body = ActionTransitionResult),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody),
    )
)]
pub async fn restore_action(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<ActionTransitionResult>> {
    transition(&state, &principal, id, Event::Restored).await
}

#[utoipa::path(
    post, path = "/v1/actions/{actionId}/completion", tag = "Actions", operation_id = "complete_action",
    summary = "Report an action done",
    description = "Report an action done, optionally with the URL of the finished work. Jam then checks the page once (status `done`, then `verified` or `unverified`). 409 for a dismissed or superseded action.",
    security(("bearerAuth" = [])),
    params(("actionId" = Uuid, Path, description = "The action id")),
    request_body = ActionCompletion,
    responses(
        (status = 200, body = ActionTransitionResult),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody),
    )
)]
pub async fn complete_action(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
    body: Option<Json<ActionCompletion>>,
) -> Result<Json<ActionTransitionResult>> {
    let body = body.map(|b| b.0).unwrap_or(ActionCompletion {
        url: None,
        note: None,
    });
    let url = check_url(body.url.as_deref())?;
    let note = body
        .note
        .as_deref()
        .map(|n| n.chars().take(1000).collect::<String>());
    let status = actions::complete(
        &state.pool,
        principal.workspace_id(),
        id,
        "user",
        url,
        note.as_deref(),
    )
    .await?;
    Ok(Json(ActionTransitionResult {
        status: status.as_str().to_owned(),
    }))
}

#[utoipa::path(
    post, path = "/v1/action-completions/{token}", tag = "Actions", operation_id = "complete_action_with_token",
    summary = "Report an action done with its token",
    description = "For the agent that did the work: the token in the action's prompt reports that one action done. No API key needed.",
    params(("token" = String, Path, description = "The token from the action's prompt: `gac_` and 32 hex digits", pattern = "^gac_[0-9a-f]{32}$")),
    request_body = ActionCompletion,
    responses(
        (status = 200, description = "Reported"),
        (status = 400, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
    )
)]
pub async fn complete_action_with_token(
    State(state): State<AppState>,
    Path(token): Path<String>,
    body: Option<Json<ActionCompletion>>,
) -> Result<()> {
    let well_formed = token.strip_prefix("gac_").is_some_and(|t| {
        t.len() == 32
            && t.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    });
    if !well_formed {
        return Err(ApiError::bad("the token must be gac_ and 32 hex digits"));
    }
    let body = body.map(|b| b.0).unwrap_or(ActionCompletion {
        url: None,
        note: None,
    });
    let url = check_url(body.url.as_deref())?;
    let note = body
        .note
        .as_deref()
        .map(|n| n.chars().take(1000).collect::<String>());
    actions::complete_with_token(&state.pool, &token, url, note.as_deref()).await?;
    Ok(())
}

/// The newest URL anyone reported for the action.
async fn last_claim_url(pool: &PgPool, id: Uuid) -> Result<Option<String>> {
    Ok(sqlx::query_scalar(
        "SELECT url FROM action_events
         WHERE action_id = $1 AND url IS NOT NULL ORDER BY id DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

#[utoipa::path(
    post, path = "/v1/actions/{actionId}/checks", tag = "Actions", operation_id = "check_action_again",
    summary = "Check an action again",
    description = "Re-verify a `done` or `unverified` action: a new done claim (at `url`, else the last claim's page) that Jam checks again. 409 when the action is not `done` or `unverified`.",
    security(("bearerAuth" = [])),
    params(("actionId" = Uuid, Path, description = "The action id")),
    request_body = ActionCheck,
    responses(
        (status = 200, body = ActionTransitionResult),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody),
    )
)]
pub async fn check_action_again(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
    body: Option<Json<ActionCheck>>,
) -> Result<Json<ActionTransitionResult>> {
    let row = load(&state, &principal, id).await?;
    let status: Status = row
        .get::<String, _>("status")
        .parse()
        .map_err(Error::Internal)?;
    if !matches!(status, Status::Done | Status::Unverified) {
        return Err(ApiError(Error::Conflict(format!(
            "a {} action cannot be checked again",
            status.as_str()
        ))));
    }
    let given = body.and_then(|b| b.0.url);
    let url = match check_url(given.as_deref())? {
        Some(u) => Some(u.to_owned()),
        None => last_claim_url(&state.pool, id).await?,
    };
    let status = actions::complete(
        &state.pool,
        principal.workspace_id(),
        id,
        "user",
        url.as_deref(),
        None,
    )
    .await?;
    Ok(Json(ActionTransitionResult {
        status: status.as_str().to_owned(),
    }))
}

#[utoipa::path(
    post, path = "/v1/actions/{actionId}/executions", tag = "Actions", operation_id = "create_action_execution",
    summary = "Have Jam do an action",
    description = "Ask Jam to do the work. An action linked to an article is drafted in the background. Other work, and the `agent` mode, is not available in this build: 422 names the reason, and `agentPrompt` tells your agent what to do. 409 when the article is held, dismissed or written.",
    security(("bearerAuth" = [])),
    params(("actionId" = Uuid, Path, description = "The action id")),
    request_body = ActionExecutionRequest,
    responses(
        (status = 202, body = ActionExecution),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody),
        (status = 422, body = ApiErrorBody),
    )
)]
pub async fn create_action_execution(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
    body: Option<Json<ActionExecutionRequest>>,
) -> Result<(StatusCode, Json<ActionExecution>)> {
    let mode = body.and_then(|b| b.0.mode);
    if !matches!(mode.as_deref(), None | Some("one_shot")) {
        return Err(ApiError(Error::Unprocessable(
            "agent mode is not available in this build; use one_shot".into(),
        )));
    }
    let a = action(&load(&state, &principal, id).await?);
    let Some(opportunity_id) = a.opportunity_id.filter(|_| a.executable_by_jam) else {
        let reason = a.not_executable_reason.unwrap_or("no_page");
        return Err(ApiError(Error::Unprocessable(format!(
            "{reason}: Jam cannot do this action; follow agentPrompt"
        ))));
    };
    start_plan(&state.pool, opportunity_id).await?;
    actions::record(
        &state.pool,
        principal.workspace_id(),
        id,
        &NewEvent::new(Event::Started, "jam"),
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(ActionExecution {
            status: "accepted",
            work: "draft",
            opportunity_id: Some(opportunity_id),
            change_id: None,
            run_id: None,
            chat_url: None,
        }),
    ))
}
