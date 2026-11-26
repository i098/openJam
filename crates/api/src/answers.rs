//! `/v1/answers` and `/v1/prompts/{promptId}/answers`: stored engine answers, newest first.

use crate::analytics::Q;
use crate::error::{ApiError, Error, Result};
use crate::{auth::Principal, error::ApiErrorBody, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

const DEFAULT_LIMIT: i64 = 25;
/// Characters of text an answer shows unless `full=true`.
const EXCERPT_CHARS: usize = 280;

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct AnswersQuery {
    /// Only answers to this prompt.
    pub prompt_id: Option<Uuid>,
    /// One engine, for example `openai`.
    pub platform: Option<String>,
    /// Only answers observed at or after this time (RFC 3339).
    pub since: Option<DateTime<Utc>>,
    /// Only answers observed at or before this time (RFC 3339).
    pub until: Option<DateTime<Utc>>,
    /// `true`: answers that cite the project's domain. `false`: answers that do not.
    pub cited: Option<bool>,
    /// Only answers that mention this competitor.
    pub competitor: Option<String>,
    /// `true`: the whole answer text. The default is a short excerpt.
    pub full: Option<bool>,
    /// Page size, 1 to 100. The default is 25.
    pub limit: Option<i64>,
    /// `nextCursor` of the page before.
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct PromptAnswersQuery {
    pub platform: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub cited: Option<bool>,
    pub competitor: Option<String>,
    pub full: Option<bool>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnswerText {
    pub text: Option<String>,
    /// `full` or `excerpt`.
    pub source: &'static str,
    pub truncated: bool,
    /// Characters in the whole answer.
    pub length: usize,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnswerBrand {
    pub name: String,
    pub owned: bool,
    pub recommended: Option<bool>,
    pub list_rank: Option<i32>,
    pub sentiment: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub id: Uuid,
    pub run_id: Uuid,
    pub prompt_id: Uuid,
    pub prompt_text: Option<String>,
    pub prompt_status: Option<String>,
    pub platform: String,
    pub observed_at: DateTime<Utc>,
    pub answer: AnswerText,
    pub brand_mentioned: Option<bool>,
    pub brand_recommended: Option<bool>,
    pub own_domain_cited: Option<bool>,
    pub position: Option<i32>,
    pub sentiment: Option<String>,
    pub brands_mentioned: Vec<AnswerBrand>,
    pub cited_urls: Vec<String>,
    pub cited_url_count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnswerPageInfo {
    pub limit: i64,
    pub has_next_page: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnswerPage {
    pub answers: Vec<Answer>,
    pub page_info: AnswerPageInfo,
    pub next_cursor: Option<String>,
}

/// Answers of the workspace, newest first, with cursor paging.
#[utoipa::path(
    get, path = "/v1/answers", tag = "Answers", operation_id = "list_answers",
    summary = "List answers", security(("bearerAuth" = [])), params(AnswersQuery),
    responses(
        (status = 200, body = AnswerPage),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody)
    )
)]
pub async fn list_answers(
    State(state): State<AppState>,
    principal: Principal,
    Q(q): Q<AnswersQuery>,
) -> Result<Json<AnswerPage>> {
    page(&state, &principal, q).await.map(Json)
}

/// Answers to one prompt, newest first.
#[utoipa::path(
    get, path = "/v1/prompts/{promptId}/answers", tag = "Answers", operation_id = "list_prompt_answers",
    summary = "List the answers to one prompt", security(("bearerAuth" = [])),
    params(("promptId" = Uuid, Path, description = "The prompt"), PromptAnswersQuery),
    responses(
        (status = 200, body = AnswerPage),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn list_prompt_answers(
    State(state): State<AppState>,
    principal: Principal,
    Path(prompt_id): Path<Uuid>,
    Q(q): Q<PromptAnswersQuery>,
) -> Result<Json<AnswerPage>> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM prompts p JOIN projects pr ON pr.id = p.project_id
                        WHERE p.id = $1 AND pr.workspace_id = $2)",
    )
    .bind(prompt_id)
    .bind(principal.workspace_id())
    .fetch_one(&state.pool)
    .await?;
    if !exists {
        return Err(ApiError(Error::NotFound("prompt")));
    }
    let q = AnswersQuery {
        prompt_id: Some(prompt_id),
        platform: q.platform,
        since: q.since,
        until: q.until,
        cited: q.cited,
        competitor: q.competitor,
        full: q.full,
        limit: q.limit,
        cursor: q.cursor,
    };
    page(&state, &principal, q).await.map(Json)
}

/// The cursor is the last answer's time in microseconds and its id.
fn parse_cursor(cursor: &str) -> Result<(i64, Uuid)> {
    cursor
        .split_once('_')
        .and_then(|(micros, id)| Some((micros.parse().ok()?, id.parse().ok()?)))
        .ok_or_else(|| ApiError::bad("cursor is not valid"))
}

type AnswerRow = (
    Uuid,
    Uuid,
    Uuid,
    String,
    String,
    String,
    DateTime<Utc>,
    String,
    Option<bool>,
    Option<bool>,
    Option<bool>,
    Option<i32>,
    Option<String>,
);

type BrandRow = (Uuid, String, bool, bool, Option<i32>, Option<String>);

async fn page(state: &AppState, principal: &Principal, q: AnswersQuery) -> Result<AnswerPage> {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=100).contains(&limit) {
        return Err(ApiError::bad("limit must be 1 to 100"));
    }
    let cursor = q.cursor.as_deref().map(parse_cursor).transpose()?;
    let rows: Vec<AnswerRow> = sqlx::query_as(
        "SELECT a.id, a.run_id, a.prompt_id, p.text, p.status, a.platform, a.observed_at, a.text,
                a.brand_mentioned, a.brand_recommended, a.own_domain_cited, a.position, a.sentiment
         FROM answers a
         JOIN prompts p ON p.id = a.prompt_id
         JOIN projects pr ON pr.id = p.project_id
         WHERE pr.workspace_id = $1
           AND ($2::uuid IS NULL OR a.prompt_id = $2)
           AND ($3::text IS NULL OR a.platform = $3)
           AND ($4::timestamptz IS NULL OR a.observed_at >= $4)
           AND ($5::timestamptz IS NULL OR a.observed_at <= $5)
           AND ($6::bool IS NULL OR COALESCE(a.own_domain_cited, false) = $6)
           AND ($7::text IS NULL OR EXISTS (
                SELECT 1 FROM answer_brands b
                WHERE b.answer_id = a.id AND NOT b.owned AND lower(b.name) = lower($7)))
           AND ($8::bigint IS NULL
                OR (a.observed_at, a.id) < (timestamptz 'epoch' + $8 * interval '1 microsecond', $9::uuid))
         ORDER BY a.observed_at DESC, a.id DESC
         LIMIT $10",
    )
    .bind(principal.workspace_id())
    .bind(q.prompt_id)
    .bind(q.platform.as_deref())
    .bind(q.since)
    .bind(q.until)
    .bind(q.cited)
    .bind(q.competitor.as_deref())
    .bind(cursor.map(|c| c.0))
    .bind(cursor.map(|c| c.1))
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;

    let has_next = rows.len() as i64 > limit;
    let rows = &rows[..rows.len().min(limit as usize)];
    let ids: Vec<Uuid> = rows.iter().map(|r| r.0).collect();

    let brand_rows: Vec<BrandRow> = sqlx::query_as(
        "SELECT answer_id, name, owned, recommended, list_rank, sentiment FROM answer_brands
         WHERE answer_id = ANY($1) ORDER BY list_rank NULLS LAST, name",
    )
    .bind(&ids)
    .fetch_all(&state.pool)
    .await?;
    let mut brands: HashMap<Uuid, Vec<AnswerBrand>> = HashMap::new();
    for (answer_id, name, owned, recommended, list_rank, sentiment) in brand_rows {
        brands.entry(answer_id).or_default().push(AnswerBrand {
            name,
            owned,
            recommended: Some(recommended),
            list_rank,
            sentiment,
        });
    }
    let url_rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT answer_id, url FROM answer_citations WHERE answer_id = ANY($1) ORDER BY id",
    )
    .bind(&ids)
    .fetch_all(&state.pool)
    .await?;
    let mut urls: HashMap<Uuid, Vec<String>> = HashMap::new();
    for (answer_id, url) in url_rows {
        urls.entry(answer_id).or_default().push(url);
    }

    let full = q.full.unwrap_or(false);
    let next_cursor = rows
        .last()
        .filter(|_| has_next)
        .map(|r| format!("{}_{}", r.6.timestamp_micros(), r.0));
    let answers = rows
        .iter()
        .map(|r| {
            let length = r.7.chars().count();
            let truncated = !full && length > EXCERPT_CHARS;
            let text = if full || !truncated {
                r.7.clone()
            } else {
                r.7.chars().take(EXCERPT_CHARS).collect()
            };
            let cited_urls = urls.remove(&r.0).unwrap_or_default();
            Answer {
                id: r.0,
                run_id: r.1,
                prompt_id: r.2,
                prompt_text: Some(r.3.clone()),
                prompt_status: Some(r.4.clone()),
                platform: r.5.clone(),
                observed_at: r.6,
                answer: AnswerText {
                    text: Some(text),
                    source: if full { "full" } else { "excerpt" },
                    truncated,
                    length,
                },
                brand_mentioned: r.8,
                brand_recommended: r.9,
                own_domain_cited: r.10,
                position: r.11,
                sentiment: r.12.clone(),
                brands_mentioned: brands.remove(&r.0).unwrap_or_default(),
                cited_url_count: cited_urls.len(),
                cited_urls,
            }
        })
        .collect();
    Ok(AnswerPage {
        answers,
        page_info: AnswerPageInfo {
            limit,
            has_next_page: has_next,
            next_cursor: next_cursor.clone(),
        },
        next_cursor,
    })
}
