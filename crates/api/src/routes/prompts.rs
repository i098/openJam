//! `/v1/prompts`: the tracked questions of a project, with their topic and status.

use crate::analytics::find_project;
use crate::auth::Principal;
use crate::error::{ApiError, ApiErrorBody, Result};
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use openjam_core::tracking::{self, NewPrompt};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 200;
/// No prompt cap in this build. The dashboard reads `-1` as "no cap".
const NO_CAP: i32 = -1;

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    pub id: Uuid,
    pub text: String,
    pub topic: Option<String>,
    pub persona: Option<String>,
    pub intent: Option<String>,
    /// `tracked` or `archived`.
    pub list: String,
    /// `active` or `paused`.
    pub status: String,
    pub branded: bool,
    pub scored: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptCounts {
    pub tracked: i64,
    pub paused: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptList {
    pub prompts: Vec<Prompt>,
    pub counts: PromptCounts,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptCap {
    /// `-1` when this install has no prompt cap.
    pub cap: i32,
    pub active_count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddedPrompt {
    pub prompt_id: Uuid,
    pub existing: bool,
    pub active_count: i64,
    pub cap: i32,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct PromptsQuery {
    pub project_id: Option<Uuid>,
    /// `tracked` (default) or `archived`.
    pub list: Option<String>,
    pub topic: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddPrompt {
    pub text: String,
    pub topic: Option<String>,
    pub persona: Option<String>,
    pub intent: Option<String>,
    #[serde(default)]
    pub branded: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BulkPrompts {
    pub prompts: Vec<AddPrompt>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BulkPromptsResult {
    pub added: i64,
    pub existing: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePrompt {
    pub text: Option<String>,
    pub topic: Option<String>,
    pub persona: Option<String>,
    pub intent: Option<String>,
    pub branded: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PausePrompt {
    pub paused: bool,
}

struct Scope {
    project_id: Uuid,
}

async fn scope(state: &AppState, principal: &Principal, project_id: Option<Uuid>) -> Result<Scope> {
    let project = find_project(state, principal, project_id).await?;
    Ok(Scope {
        project_id: project.id,
    })
}

async fn counts(state: &AppState, project_id: Uuid) -> Result<PromptCounts> {
    type Row = (i64, i64);
    let (tracked, paused): Row = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE list = 'tracked'), count(*) FILTER (WHERE status = 'paused')
         FROM prompts WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(PromptCounts { tracked, paused })
}

async fn load(state: &AppState, project_id: Uuid, prompt_id: Uuid) -> Result<Prompt> {
    sqlx::query_as(
        "SELECT id, text, topic, persona, intent, list, status, branded, scored, updated_at
         FROM prompts WHERE id = $1 AND project_id = $2",
    )
    .bind(prompt_id)
    .bind(project_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(ApiError(crate::error::Error::NotFound("prompt")))
}

fn clean_text(text: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 500 {
        return Err(ApiError::bad("a prompt needs 1 to 500 characters"));
    }
    Ok(text.to_owned())
}

/// Add one prompt. A prompt with the same text on the same project is not added twice.
async fn insert(state: &AppState, project_id: Uuid, body: &AddPrompt) -> Result<(Uuid, bool)> {
    let text = clean_text(&body.text)?;
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM prompts WHERE project_id = $1 AND list = 'tracked' AND lower(text) = lower($2) LIMIT 1",
    )
    .bind(project_id)
    .bind(&text)
    .fetch_optional(&state.pool)
    .await?;
    if let Some(id) = existing {
        return Ok((id, true));
    }
    let id = tracking::add_prompt(
        &state.pool,
        project_id,
        &NewPrompt {
            text,
            topic: body.topic.clone().filter(|t| !t.trim().is_empty()),
            persona: body.persona.clone().filter(|p| !p.trim().is_empty()),
            intent: body.intent.clone().filter(|i| !i.trim().is_empty()),
            branded: body.branded,
        },
    )
    .await?;
    Ok((id, false))
}

#[utoipa::path(
    get, path = "/v1/prompts", tag = "Prompts", operation_id = "list_prompts",
    summary = "List prompts", security(("bearerAuth" = [])), params(PromptsQuery),
    responses((status = 200, body = PromptList), (status = 401, body = ApiErrorBody))
)]
pub async fn list_prompts(
    State(state): State<AppState>,
    principal: Principal,
    axum::extract::Query(query): axum::extract::Query<PromptsQuery>,
) -> Result<Json<PromptList>> {
    let scope = scope(&state, &principal, query.project_id).await?;
    let list = match query.list.as_deref() {
        Some("archived") => "archived",
        _ => "tracked",
    };
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let prompts: Vec<Prompt> = sqlx::query_as(
        "SELECT id, text, topic, persona, intent, list, status, branded, scored, updated_at
         FROM prompts
         WHERE project_id = $1 AND list = $2 AND ($3::text IS NULL OR topic = $3)
         ORDER BY created_at, id LIMIT $4",
    )
    .bind(scope.project_id)
    .bind(list)
    .bind(query.topic)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(PromptList {
        prompts,
        counts: counts(&state, scope.project_id).await?,
    }))
}

#[utoipa::path(
    post, path = "/v1/prompts", tag = "Prompts", operation_id = "add_prompt",
    summary = "Add a prompt", security(("bearerAuth" = [])),
    params(("projectId" = Option<Uuid>, Query, description = "The project; newest when omitted")),
    request_body = AddPrompt,
    responses(
        (status = 201, body = AddedPrompt),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody)
    )
)]
pub async fn add_prompt(
    State(state): State<AppState>,
    principal: Principal,
    axum::extract::Query(query): axum::extract::Query<SingleProject>,
    Json(body): Json<AddPrompt>,
) -> Result<(StatusCode, Json<AddedPrompt>)> {
    let scope = scope(&state, &principal, query.project_id).await?;
    let (prompt_id, existing) = insert(&state, scope.project_id, &body).await?;
    let counts = counts(&state, scope.project_id).await?;
    Ok((
        if existing {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        Json(AddedPrompt {
            prompt_id,
            existing,
            active_count: counts.tracked,
            cap: NO_CAP,
        }),
    ))
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct SingleProject {
    pub project_id: Option<Uuid>,
}

#[utoipa::path(
    post, path = "/v1/prompts/bulk", tag = "Prompts", operation_id = "add_prompts_bulk",
    summary = "Add several prompts", security(("bearerAuth" = [])),
    params(("projectId" = Option<Uuid>, Query, description = "The project; newest when omitted")),
    request_body = BulkPrompts,
    responses(
        (status = 200, body = BulkPromptsResult),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody)
    )
)]
pub async fn add_prompts_bulk(
    State(state): State<AppState>,
    principal: Principal,
    axum::extract::Query(query): axum::extract::Query<SingleProject>,
    Json(body): Json<BulkPrompts>,
) -> Result<Json<BulkPromptsResult>> {
    let scope = scope(&state, &principal, query.project_id).await?;
    if body.prompts.len() > 500 {
        return Err(ApiError::bad("at most 500 prompts in one call"));
    }
    let mut added = 0;
    let mut existing = 0;
    for prompt in &body.prompts {
        let (_, was_existing) = insert(&state, scope.project_id, prompt).await?;
        if was_existing {
            existing += 1;
        } else {
            added += 1;
        }
    }
    Ok(Json(BulkPromptsResult { added, existing }))
}

#[utoipa::path(
    get, path = "/v1/prompts/cap", tag = "Prompts", operation_id = "get_prompt_cap",
    summary = "The prompt cap of this install", security(("bearerAuth" = [])),
    params(SingleProject),
    responses((status = 200, body = PromptCap), (status = 401, body = ApiErrorBody))
)]
pub async fn get_prompt_cap(
    State(state): State<AppState>,
    principal: Principal,
    axum::extract::Query(query): axum::extract::Query<SingleProject>,
) -> Result<Json<PromptCap>> {
    let scope = scope(&state, &principal, query.project_id).await?;
    Ok(Json(PromptCap {
        cap: NO_CAP,
        active_count: counts(&state, scope.project_id).await?.tracked,
    }))
}

#[utoipa::path(
    get, path = "/v1/prompts/{promptId}", tag = "Prompts", operation_id = "get_prompt",
    summary = "Get one prompt", security(("bearerAuth" = [])),
    params(("promptId" = Uuid, Path, description = "The prompt")),
    responses(
        (status = 200, body = Prompt),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn get_prompt(
    State(state): State<AppState>,
    principal: Principal,
    Path(prompt_id): Path<Uuid>,
) -> Result<Json<Prompt>> {
    let prompt = sqlx::query_as(
        "SELECT id, text, topic, persona, intent, list, status, branded, scored, updated_at
         FROM prompts WHERE id = $1 AND project_id IN
         (SELECT id FROM projects WHERE workspace_id = $2 AND archived_at IS NULL)",
    )
    .bind(prompt_id)
    .bind(principal.workspace_id())
    .fetch_optional(&state.pool)
    .await?
    .ok_or(crate::error::Error::NotFound("prompt"))?;
    Ok(Json(prompt))
}

#[utoipa::path(
    patch, path = "/v1/prompts/{promptId}", tag = "Prompts", operation_id = "edit_prompt",
    summary = "Edit a prompt", security(("bearerAuth" = [])),
    params(("promptId" = Uuid, Path, description = "The prompt")), request_body = UpdatePrompt,
    responses(
        (status = 200, body = Prompt),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn edit_prompt(
    State(state): State<AppState>,
    principal: Principal,
    Path(prompt_id): Path<Uuid>,
    Json(body): Json<UpdatePrompt>,
) -> Result<Json<Prompt>> {
    let project_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT project_id FROM prompts WHERE id = $1 AND project_id IN
         (SELECT id FROM projects WHERE workspace_id = $2)",
    )
    .bind(prompt_id)
    .bind(principal.workspace_id())
    .fetch_optional(&state.pool)
    .await?;
    let Some(project_id) = project_id else {
        return Err(crate::error::Error::NotFound("prompt").into());
    };
    let text = body.text.as_deref().map(clean_text).transpose()?;
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE prompts SET
             text = COALESCE($2, text),
             topic = CASE WHEN $3 THEN $4 ELSE topic END,
             persona = CASE WHEN $5 THEN $6 ELSE persona END,
             intent = CASE WHEN $7 THEN $8 ELSE intent END,
             branded = COALESCE($9, branded),
             updated_at = now()
         WHERE id = $1",
    )
    .bind(prompt_id)
    .bind(&text)
    .bind(body.topic.is_some())
    .bind(
        body.topic
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty()),
    )
    .bind(body.persona.is_some())
    .bind(
        body.persona
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty()),
    )
    .bind(body.intent.is_some())
    .bind(
        body.intent
            .map(|i| i.trim().to_owned())
            .filter(|i| !i.is_empty()),
    )
    .bind(body.branded)
    .execute(&mut *tx)
    .await?;
    if let Some(text) = &text {
        sqlx::query("INSERT INTO prompt_versions (prompt_id, text) VALUES ($1, $2)")
            .bind(prompt_id)
            .bind(text)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(load(&state, project_id, prompt_id).await?))
}

#[utoipa::path(
    delete, path = "/v1/prompts/{promptId}", tag = "Prompts", operation_id = "remove_prompt",
    summary = "Remove a prompt", security(("bearerAuth" = [])),
    params(("promptId" = Uuid, Path, description = "The prompt")),
    responses(
        (status = 204),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn remove_prompt(
    State(state): State<AppState>,
    principal: Principal,
    Path(prompt_id): Path<Uuid>,
) -> Result<StatusCode> {
    let deleted = sqlx::query(
        "UPDATE prompts SET list = 'archived', updated_at = now() WHERE id = $1 AND project_id IN
         (SELECT id FROM projects WHERE workspace_id = $2 AND archived_at IS NULL)",
    )
    .bind(prompt_id)
    .bind(principal.workspace_id())
    .execute(&state.pool)
    .await?
    .rows_affected();
    if deleted == 0 {
        return Err(crate::error::Error::NotFound("prompt").into());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post, path = "/v1/prompts/{promptId}/pause", tag = "Prompts", operation_id = "pause_prompt",
    summary = "Pause or resume a prompt", security(("bearerAuth" = [])),
    params(("promptId" = Uuid, Path, description = "The prompt")), request_body = PausePrompt,
    responses(
        (status = 200, body = Prompt),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn pause_prompt(
    State(state): State<AppState>,
    principal: Principal,
    Path(prompt_id): Path<Uuid>,
    Json(body): Json<PausePrompt>,
) -> Result<Json<Prompt>> {
    let project_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT project_id FROM prompts WHERE id = $1 AND project_id IN
         (SELECT id FROM projects WHERE workspace_id = $2)",
    )
    .bind(prompt_id)
    .bind(principal.workspace_id())
    .fetch_optional(&state.pool)
    .await?;
    let Some(project_id) = project_id else {
        return Err(crate::error::Error::NotFound("prompt").into());
    };
    sqlx::query(
        "UPDATE prompts SET status = $2, paused_reason = $3, updated_at = now() WHERE id = $1",
    )
    .bind(prompt_id)
    .bind(if body.paused { "paused" } else { "active" })
    .bind(body.paused.then_some("user"))
    .execute(&state.pool)
    .await?;
    Ok(Json(load(&state, project_id, prompt_id).await?))
}
