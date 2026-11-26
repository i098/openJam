//! `/v1/content`: drafts with revisions, discard and restore, rewrite requests and Markdown export.

use crate::analytics::Q;
use crate::error::{ApiError, ApiErrorBody, Error, Result};
use crate::{auth::Principal, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use openjam_core::jobs;
use openjam_publish::{
    article::Article,
    store::{QUEUE_UPDATE, insert_revision},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, PgPool, types::Json as Jsonb};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

const DEFAULT_LIMIT: i64 = 100;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Finding {
    pub check: String,
    pub code: String,
    /// `blocking` or `advisory`.
    pub severity: String,
    pub message: String,
    pub quote: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RevisionSummary {
    pub id: Uuid,
    pub title: String,
    /// Who wrote it: `pipeline` (openJam), `human` or `agent`.
    pub source: String,
    pub basis_revision_id: Option<Uuid>,
    pub evidence_bundle_id: Option<Uuid>,
    pub findings: Vec<Finding>,
    pub needs_review: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Revision {
    #[serde(flatten)]
    pub summary: RevisionSummary,
    pub content_id: Uuid,
    pub article: Article,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateFailure {
    /// `insufficient_credits`, `agent_failed` or `failed`.
    pub code: String,
    pub message: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Content {
    pub id: Uuid,
    pub project_id: Uuid,
    pub opportunity_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub discarded_at: Option<DateTime<Utc>>,
    /// The last failure to regenerate or edit it, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_failure: Option<UpdateFailure>,
    pub current_revision: RevisionSummary,
    /// `drafted` or `discarded`.
    pub state: String,
    /// While drafted: whether the last update failed after the current revision was stored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_failed: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContentPageInfo {
    pub limit: i64,
    pub has_next_page: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub total_count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContentList {
    pub content: Vec<Content>,
    pub page_info: ContentPageInfo,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContentItem {
    pub content: Content,
    pub current_revision: Revision,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreatedContent {
    pub content_id: Uuid,
    pub revision_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddedRevision {
    pub revision_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Accepted {
    /// Always `accepted`.
    pub status: &'static str,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateContent {
    /// An id from `GET /v1/opportunities`.
    pub opportunity_id: Uuid,
    pub article: Article,
    /// The evidence bundle the draft was written from, if any.
    pub bundle_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddRevision {
    pub article: Article,
    /// The revision this one was written from, if any.
    pub basis_revision_id: Option<Uuid>,
    /// The evidence bundle it was written from, if any.
    pub bundle_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegenerateContent {
    /// The revision being replaced, normally the current one.
    pub basis_revision_id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EditRequest {
    /// The revision to edit, normally the current one.
    pub basis_revision_id: Uuid,
    /// What to change, at most 4000 characters.
    pub instruction: String,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// Only content in these states: `drafted`, `discarded`, or both separated by a comma.
    pub state: Option<String>,
    /// Items per page, 1 to 100. The default is 100.
    pub limit: Option<i64>,
    /// `pageInfo.nextCursor` of the page before.
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct MarkdownQuery {
    /// The revision to export. The default is the current one.
    pub revision_id: Option<Uuid>,
}

/// One content row joined with its current revision.
#[derive(FromRow)]
struct ContentRow {
    id: Uuid,
    project_id: Uuid,
    opportunity_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    discarded_at: Option<DateTime<Utc>>,
    state: String,
    update_failure: Option<Jsonb<UpdateFailure>>,
    rev_id: Uuid,
    title: String,
    source: String,
    basis_revision_id: Option<Uuid>,
    evidence_bundle_id: Option<Uuid>,
    findings: Jsonb<Vec<Finding>>,
    needs_review: bool,
    rev_created_at: DateTime<Utc>,
}

const SELECT_CONTENT: &str =
    "SELECT c.id, c.project_id, c.opportunity_id, c.created_at, c.discarded_at,
        c.state, c.update_failure, r.id AS rev_id, r.title, r.source, r.basis_revision_id,
        r.evidence_bundle_id, r.findings, r.needs_review, r.created_at AS rev_created_at
    FROM contents c
    JOIN content_revisions r ON r.id = c.current_revision_id
    JOIN projects p ON p.id = c.project_id";

impl ContentRow {
    fn summary(&self) -> RevisionSummary {
        RevisionSummary {
            id: self.rev_id,
            title: self.title.clone(),
            source: self.source.clone(),
            basis_revision_id: self.basis_revision_id,
            evidence_bundle_id: self.evidence_bundle_id,
            findings: Vec::new(),
            needs_review: self.needs_review,
            created_at: self.rev_created_at,
        }
    }

    fn api(self) -> Content {
        let drafted = self.state == "drafted";
        let mut current = self.summary();
        current.findings = self.findings.0;
        Content {
            id: self.id,
            project_id: self.project_id,
            opportunity_id: self.opportunity_id,
            created_at: self.created_at,
            discarded_at: self.discarded_at,
            update_failed: drafted.then_some(self.update_failure.is_some()),
            update_failure: self.update_failure.map(|j| j.0),
            current_revision: current,
            state: self.state,
        }
    }
}

#[derive(FromRow)]
struct RevisionRow {
    id: Uuid,
    content_id: Uuid,
    title: String,
    source: String,
    basis_revision_id: Option<Uuid>,
    evidence_bundle_id: Option<Uuid>,
    findings: Jsonb<Vec<Finding>>,
    needs_review: bool,
    created_at: DateTime<Utc>,
    article: Jsonb<Article>,
}

const SELECT_REVISION: &str = "SELECT r.id, r.content_id, r.title, r.source, r.basis_revision_id,
        r.evidence_bundle_id, r.findings, r.needs_review, r.created_at, r.article
    FROM content_revisions r
    JOIN contents c ON c.id = r.content_id
    JOIN projects p ON p.id = c.project_id";

impl RevisionRow {
    fn summary(&self) -> RevisionSummary {
        RevisionSummary {
            id: self.id,
            title: self.title.clone(),
            source: self.source.clone(),
            basis_revision_id: self.basis_revision_id,
            evidence_bundle_id: self.evidence_bundle_id,
            findings: Vec::new(),
            needs_review: self.needs_review,
            created_at: self.created_at,
        }
    }

    fn api(self) -> Revision {
        let mut summary = self.summary();
        summary.findings = self.findings.0;
        Revision {
            summary,
            content_id: self.content_id,
            article: self.article.0,
        }
    }
}

/// The project a request works on: the `X-Jam-Project` header, else the newest project.
pub async fn active_project(
    pool: &PgPool,
    principal: &Principal,
    headers: &HeaderMap,
) -> Result<Uuid> {
    let wanted = match headers.get("x-jam-project") {
        None => None,
        Some(v) => Some(
            v.to_str()
                .ok()
                .and_then(|s| s.parse::<Uuid>().ok())
                .ok_or_else(|| ApiError::bad("X-Jam-Project must be a project id"))?,
        ),
    };
    sqlx::query_scalar(
        "SELECT id FROM projects
         WHERE workspace_id = $1 AND archived_at IS NULL AND ($2::uuid IS NULL OR id = $2)
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(principal.workspace_id())
    .bind(wanted)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError(Error::NotFound("project")))
}

/// Fail with 404 unless the content item is in the caller's workspace.
pub async fn require_content(pool: &PgPool, principal: &Principal, id: Uuid) -> Result<()> {
    let found: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM contents c JOIN projects p ON p.id = c.project_id
                        WHERE c.id = $1 AND p.workspace_id = $2)",
    )
    .bind(id)
    .bind(principal.workspace_id())
    .fetch_one(pool)
    .await?;
    found
        .then_some(())
        .ok_or(ApiError(Error::NotFound("content")))
}

async fn load(pool: &PgPool, principal: &Principal, id: Uuid) -> Result<ContentRow> {
    sqlx::query_as(&format!(
        "{SELECT_CONTENT} WHERE c.id = $1 AND p.workspace_id = $2"
    ))
    .bind(id)
    .bind(principal.workspace_id())
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError(Error::NotFound("content")))
}

fn check(article: &Article) -> Result<()> {
    article.check().map_err(ApiError::bad)
}

fn source(principal: &Principal) -> &'static str {
    match principal {
        Principal::Session(_) => "human",
        Principal::Key(_) => "agent",
    }
}

/// Store an agent's article as an opportunity's content, with its first revision.
#[utoipa::path(
    post, path = "/v1/content", tag = "Content", operation_id = "create_content",
    summary = "Store a draft", security(("bearerAuth" = [])), request_body = CreateContent,
    responses(
        (status = 201, body = CreatedContent),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody)
    )
)]
pub async fn create_content(
    State(state): State<AppState>,
    principal: Principal,
    Json(body): Json<CreateContent>,
) -> Result<(StatusCode, Json<CreatedContent>)> {
    check(&body.article)?;
    let project: Uuid = sqlx::query_scalar(
        "SELECT o.project_id FROM opportunities o JOIN projects p ON p.id = o.project_id
         WHERE o.id = $1 AND p.workspace_id = $2",
    )
    .bind(body.opportunity_id)
    .bind(principal.workspace_id())
    .fetch_optional(&state.pool)
    .await?
    .ok_or(ApiError(Error::NotFound("opportunity")))?;

    let mut tx = state.pool.begin().await?;
    // The opportunity row lock makes two drafts for one opportunity run one after the other.
    sqlx::query("SELECT id FROM opportunities WHERE id = $1 FOR UPDATE")
        .bind(body.opportunity_id)
        .execute(&mut *tx)
        .await?;
    let used: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM contents WHERE opportunity_id = $1 AND state = 'drafted')",
    )
    .bind(body.opportunity_id)
    .fetch_one(&mut *tx)
    .await?;
    if used {
        return Err(ApiError(Error::Conflict(
            "this opportunity already has drafted content".into(),
        )));
    }
    let content_id: Uuid = sqlx::query_scalar(
        "INSERT INTO contents (project_id, opportunity_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(project)
    .bind(body.opportunity_id)
    .fetch_one(&mut *tx)
    .await?;
    let revision_id = insert_revision(
        &mut tx,
        content_id,
        &body.article,
        source(&principal),
        None,
        body.bundle_id,
    )
    .await?;
    sqlx::query("UPDATE opportunities SET claim = NULL WHERE id = $1")
        .bind(body.opportunity_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedContent {
            content_id,
            revision_id,
        }),
    ))
}

/// The project's content, newest first, with cursor paging.
#[utoipa::path(
    get, path = "/v1/content", tag = "Content", operation_id = "list_content",
    summary = "List content", security(("bearerAuth" = [])), params(ListQuery),
    responses(
        (status = 200, body = ContentList),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn list_content(
    State(state): State<AppState>,
    principal: Principal,
    headers: HeaderMap,
    Q(q): Q<ListQuery>,
) -> Result<Json<ContentList>> {
    let project = active_project(&state.pool, &principal, &headers).await?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=100).contains(&limit) {
        return Err(ApiError::bad("limit must be 1 to 100"));
    }
    let states: Vec<String> = match &q.state {
        None => vec!["drafted".into(), "discarded".into()],
        Some(s) => s.split(',').map(|s| s.trim().to_owned()).collect(),
    };
    if states.iter().any(|s| s != "drafted" && s != "discarded") {
        return Err(ApiError::bad("state must be drafted, discarded or both"));
    }
    let after = q.cursor.as_deref().map(parse_cursor).transpose()?;
    let (after_micros, after_id) = after.unzip();
    let rows: Vec<ContentRow> = sqlx::query_as(&format!(
        "{SELECT_CONTENT}
         WHERE c.project_id = $1 AND c.state = ANY($2)
           AND ($3::bigint IS NULL
                OR (floor(extract(epoch FROM c.created_at) * 1000000)::bigint, c.id) < ($3, $4::uuid))
         ORDER BY c.created_at DESC, c.id DESC LIMIT $5"
    ))
    .bind(project)
    .bind(&states)
    .bind(after_micros)
    .bind(after_id)
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM contents WHERE project_id = $1 AND state = ANY($2)",
    )
    .bind(project)
    .bind(&states)
    .fetch_one(&state.pool)
    .await?;
    let has_next_page = rows.len() as i64 > limit;
    let mut rows = rows;
    rows.truncate(limit as usize);
    let next_cursor = has_next_page
        .then(|| {
            rows.last()
                .map(|r| format!("{}_{}", r.created_at.timestamp_micros(), r.id))
        })
        .flatten();
    Ok(Json(ContentList {
        content: rows.into_iter().map(ContentRow::api).collect(),
        page_info: ContentPageInfo {
            limit,
            has_next_page,
            next_cursor,
            total_count: total,
        },
    }))
}

fn parse_cursor(cursor: &str) -> Result<(i64, Uuid)> {
    cursor
        .split_once('_')
        .and_then(|(micros, id)| Some((micros.parse().ok()?, id.parse().ok()?)))
        .ok_or_else(|| ApiError::bad("cursor is not valid"))
}

/// One content item with its state and current revision.
#[utoipa::path(
    get, path = "/v1/content/{contentId}", tag = "Content", operation_id = "get_content",
    summary = "Get content", security(("bearerAuth" = [])),
    params(("contentId" = Uuid, Path, description = "The content item")),
    responses(
        (status = 200, body = ContentItem),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn get_content(
    State(state): State<AppState>,
    principal: Principal,
    Path(content_id): Path<Uuid>,
) -> Result<Json<ContentItem>> {
    let row = load(&state.pool, &principal, content_id).await?;
    let revision = load_revision(&state.pool, &principal, content_id, row.rev_id).await?;
    Ok(Json(ContentItem {
        content: row.api(),
        current_revision: revision.api(),
    }))
}

async fn load_revision(
    pool: &PgPool,
    principal: &Principal,
    content_id: Uuid,
    revision_id: Uuid,
) -> Result<RevisionRow> {
    sqlx::query_as(&format!(
        "{SELECT_REVISION} WHERE r.id = $1 AND r.content_id = $2 AND p.workspace_id = $3"
    ))
    .bind(revision_id)
    .bind(content_id)
    .bind(principal.workspace_id())
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError(Error::NotFound("revision")))
}

/// Discard a content item. Repeating it changes nothing.
#[utoipa::path(
    delete, path = "/v1/content/{contentId}", tag = "Content", operation_id = "discard_content",
    summary = "Discard content", security(("bearerAuth" = [])),
    params(("contentId" = Uuid, Path, description = "The content item")),
    responses(
        (status = 204, description = "The content is discarded."),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn discard_content(
    State(state): State<AppState>,
    principal: Principal,
    Path(content_id): Path<Uuid>,
) -> Result<StatusCode> {
    require_content(&state.pool, &principal, content_id).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE contents SET state = 'discarded', discarded_at = now()
         WHERE id = $1 AND state = 'drafted'",
    )
    .bind(content_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE deliveries SET status = 'cancelled', cancelled_at = now()
         WHERE content_id = $1 AND status = 'scheduled'",
    )
    .bind(content_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Undo a discard. Repeating it changes nothing.
#[utoipa::path(
    post, path = "/v1/content/{contentId}/restorations", tag = "Content", operation_id = "restore_content",
    summary = "Restore discarded content", security(("bearerAuth" = [])),
    params(("contentId" = Uuid, Path, description = "The content item")),
    responses(
        (status = 204, description = "The content is restored."),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn restore_content(
    State(state): State<AppState>,
    principal: Principal,
    Path(content_id): Path<Uuid>,
) -> Result<StatusCode> {
    require_content(&state.pool, &principal, content_id).await?;
    sqlx::query(
        "UPDATE contents SET state = 'drafted', discarded_at = NULL
         WHERE id = $1 AND state = 'discarded'",
    )
    .bind(content_id)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Queue a rewrite of one revision: the worker writes it, then it becomes the current draft.
async fn queue_update(
    state: &AppState,
    principal: &Principal,
    content_id: Uuid,
    basis: Uuid,
    instruction: Option<&str>,
) -> Result<(StatusCode, Json<Accepted>)> {
    let content = load(&state.pool, principal, content_id).await?;
    if content.state != "drafted" {
        return Err(ApiError(Error::Conflict(
            "the content is discarded; restore it first".into(),
        )));
    }
    load_revision(&state.pool, principal, content_id, basis).await?;
    jobs::enqueue(
        &state.pool,
        QUEUE_UPDATE,
        &json!({"contentId": content_id, "basisRevisionId": basis, "instruction": instruction}),
        None,
        None,
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(Accepted { status: "accepted" })))
}

/// Write the content again. It runs in the background.
#[utoipa::path(
    post, path = "/v1/content/{contentId}/regenerations", tag = "Content", operation_id = "regenerate_content",
    summary = "Write the content again", security(("bearerAuth" = [])), request_body = RegenerateContent,
    params(("contentId" = Uuid, Path, description = "The content item")),
    responses(
        (status = 202, body = Accepted),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody)
    )
)]
pub async fn regenerate_content(
    State(state): State<AppState>,
    principal: Principal,
    Path(content_id): Path<Uuid>,
    Json(body): Json<RegenerateContent>,
) -> Result<(StatusCode, Json<Accepted>)> {
    queue_update(&state, &principal, content_id, body.basis_revision_id, None).await
}

/// Ask for an edit of one revision. It runs in the background.
#[utoipa::path(
    post, path = "/v1/content/{contentId}/edit-requests", tag = "Content", operation_id = "request_content_edit",
    summary = "Request an edit", security(("bearerAuth" = [])), request_body = EditRequest,
    params(("contentId" = Uuid, Path, description = "The content item")),
    responses(
        (status = 202, body = Accepted),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody)
    )
)]
pub async fn request_content_edit(
    State(state): State<AppState>,
    principal: Principal,
    Path(content_id): Path<Uuid>,
    Json(body): Json<EditRequest>,
) -> Result<(StatusCode, Json<Accepted>)> {
    let instruction = body.instruction.trim();
    if instruction.is_empty() || instruction.chars().count() > 4000 {
        return Err(ApiError::bad("instruction must be 1 to 4000 characters"));
    }
    queue_update(
        &state,
        &principal,
        content_id,
        body.basis_revision_id,
        Some(instruction),
    )
    .await
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Revisions {
    pub revisions: Vec<RevisionSummary>,
}

/// The revisions of a content item without their articles, newest first.
#[utoipa::path(
    get, path = "/v1/content/{contentId}/revisions", tag = "Revisions", operation_id = "list_content_revisions",
    summary = "List revisions", security(("bearerAuth" = [])),
    params(("contentId" = Uuid, Path, description = "The content item")),
    responses(
        (status = 200, body = Revisions),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn list_content_revisions(
    State(state): State<AppState>,
    principal: Principal,
    Path(content_id): Path<Uuid>,
) -> Result<Json<Revisions>> {
    require_content(&state.pool, &principal, content_id).await?;
    let rows: Vec<RevisionRow> = sqlx::query_as(&format!(
        "{SELECT_REVISION} WHERE r.content_id = $1 AND p.workspace_id = $2 ORDER BY r.n DESC"
    ))
    .bind(content_id)
    .bind(principal.workspace_id())
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(Revisions {
        revisions: rows
            .into_iter()
            .map(|r| {
                let mut s = r.summary();
                s.findings = r.findings.0;
                s
            })
            .collect(),
    }))
}

/// Store an article as a new revision. It becomes the current draft.
#[utoipa::path(
    post, path = "/v1/content/{contentId}/revisions", tag = "Revisions", operation_id = "add_content_revision",
    summary = "Store a revision", security(("bearerAuth" = [])), request_body = AddRevision,
    params(("contentId" = Uuid, Path, description = "The content item")),
    responses(
        (status = 201, body = AddedRevision),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody)
    )
)]
pub async fn add_content_revision(
    State(state): State<AppState>,
    principal: Principal,
    Path(content_id): Path<Uuid>,
    Json(body): Json<AddRevision>,
) -> Result<(StatusCode, Json<AddedRevision>)> {
    check(&body.article)?;
    require_content(&state.pool, &principal, content_id).await?;
    if let Some(basis) = body.basis_revision_id {
        load_revision(&state.pool, &principal, content_id, basis).await?;
    }
    let mut tx = state.pool.begin().await?;
    let content_state: String =
        sqlx::query_scalar("SELECT state FROM contents WHERE id = $1 FOR UPDATE")
            .bind(content_id)
            .fetch_one(&mut *tx)
            .await?;
    if content_state != "drafted" {
        return Err(ApiError(Error::Conflict(
            "the content is discarded; restore it first".into(),
        )));
    }
    let revision_id = insert_revision(
        &mut tx,
        content_id,
        &body.article,
        source(&principal),
        body.basis_revision_id,
        body.bundle_id,
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(AddedRevision { revision_id })))
}

/// One revision with its article.
#[utoipa::path(
    get, path = "/v1/content/{contentId}/revisions/{revisionId}", tag = "Revisions", operation_id = "get_content_revision",
    summary = "Get a revision", security(("bearerAuth" = [])),
    params(
        ("contentId" = Uuid, Path, description = "The content item"),
        ("revisionId" = Uuid, Path, description = "The revision")
    ),
    responses(
        (status = 200, body = Revision),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn get_content_revision(
    State(state): State<AppState>,
    principal: Principal,
    Path((content_id, revision_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Revision>> {
    Ok(Json(
        load_revision(&state.pool, &principal, content_id, revision_id)
            .await?
            .api(),
    ))
}

/// The article as one Markdown file: YAML frontmatter, then the body.
#[utoipa::path(
    get, path = "/v1/content/{contentId}/markdown", tag = "Content", operation_id = "export_content_markdown",
    summary = "Export as Markdown", security(("bearerAuth" = [])),
    params(("contentId" = Uuid, Path, description = "The content item"), MarkdownQuery),
    responses(
        (status = 200, content_type = "text/markdown", body = String),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn export_content_markdown(
    State(state): State<AppState>,
    principal: Principal,
    Path(content_id): Path<Uuid>,
    Q(q): Q<MarkdownQuery>,
) -> Result<Response> {
    let revision = match q.revision_id {
        Some(id) => id,
        None => load(&state.pool, &principal, content_id).await?.rev_id,
    };
    let revision = load_revision(&state.pool, &principal, content_id, revision).await?;
    let file = revision.article.0.to_markdown(Utc::now().date_naive());
    let disposition = format!("attachment; filename=\"{}.md\"", revision.article.0.slug);
    Ok((
        [
            (
                header::CONTENT_TYPE,
                "text/markdown; charset=utf-8".to_owned(),
            ),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        file,
    )
        .into_response())
}
