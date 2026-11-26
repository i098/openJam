//! `/v1/opportunities*`: the article pipeline behind the Plan's page actions. An agent can claim an
//! opportunity and write it from the brief; Jam writes it when the content worker takes the plan job.

use crate::analytics::{Q, find_project, types::PageInfo};
use crate::error::{ApiError, ApiErrorBody, Error, Result};
use crate::{auth::Principal, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, Utc};
use openjam_core::{jobs, research::QUEUE_PLAN};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Row, postgres::PgRow};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

const DEFAULT_LIMIT: i64 = 100;
/// An agent holds an opportunity this long.
const CLAIM_HOURS: i64 = 24;
const STATES: [&str; 6] = [
    "open",
    "planned",
    "writing",
    "claimed",
    "creation_failed",
    "dismissed",
];

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Opportunity {
    pub id: Uuid,
    pub project_id: Uuid,
    /// `create_page` or `update_page`.
    pub kind: String,
    /// For an update: what to change on the page.
    pub change_kind: Option<String>,
    /// For an update: the page it changes.
    pub target_page_id: Option<String>,
    pub title: String,
    pub rationale: String,
    pub topic: String,
    pub persona: String,
    pub intent: String,
    pub score: f64,
    pub created_at: DateTime<Utc>,
    /// `open`, `planned`, `writing`, `claimed`, `creation_failed` or `dismissed`.
    pub state: String,
    pub dismissed_at: Option<DateTime<Utc>>,
    /// The last failure to create content from it: `{code, message, at}`.
    pub creation_failure: Option<Value>,
    /// The content created from it, if any.
    pub content_id: Option<Uuid>,
    /// Who holds it now: a Jam run (`pipeline`) or an agent (`agent`).
    pub claim: Option<ClaimView>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClaimView {
    pub kind: String,
    pub claimed_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Opportunities {
    pub opportunities: Vec<Opportunity>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityDetail {
    #[serde(flatten)]
    pub opportunity: Opportunity,
    pub prompts: Vec<EvidencePrompt>,
    /// Always empty: openJam has no demand queries.
    pub demand_queries: Vec<Value>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EvidencePrompt {
    pub prompt_id: String,
    pub prompt_version_id: Option<String>,
    pub prompt_run_id: Option<String>,
    pub platform: Option<String>,
    /// `absent` (the brand is in no answer) or `weak`.
    pub evidence_kind: String,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// One or more states, comma separated.
    pub state: Option<String>,
    /// Items per page, 1 to 100. The default is 100.
    pub limit: Option<i64>,
    /// `pageInfo.nextCursor` of the page before.
    pub cursor: Option<String>,
}

/// Send `{}`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct Claim {}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClaimOpened {
    pub claim_id: Uuid,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AgentFailure {
    /// Why the article could not be written, in words a person can read.
    pub reason: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Accepted {
    pub status: &'static str,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityBrief {
    pub opportunity_id: Uuid,
    pub kind: String,
    pub title: String,
    pub rationale: String,
    pub topic: String,
    pub persona: String,
    pub intent: String,
    /// The questions users ask that this article should answer.
    pub target_prompts: Vec<String>,
    pub voice: Voice,
    pub research: BriefResearch,
    /// `null` for a page update, or when no plan was stored.
    pub article_plan: Option<Value>,
    /// Always 0: the plan comes with the research.
    pub credits_charged: i32,
    pub plan_unavailable: Option<String>,
    /// The destination of the article. `null` until the project has one.
    pub delivery: Option<Value>,
    pub submit: Submit,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Voice {
    pub brand: BrandVoice,
    pub language: Language,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrandVoice {
    pub name: String,
    pub positioning: Option<String>,
    pub audience: Option<String>,
    pub product_description: Option<String>,
    pub tone: Option<String>,
    pub content_style: Option<String>,
    pub excluded_topics: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Language {
    pub code: String,
    pub name: String,
    pub native_name: String,
    pub country: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BriefResearch {
    pub evidence_bundle_id: String,
    /// The pages research read and what it recorded from each.
    pub sources: Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Submit {
    pub method: &'static str,
    pub path: &'static str,
    pub body: Value,
}

fn parse_states(raw: Option<&str>) -> Result<Option<Vec<String>>> {
    let Some(raw) = raw else { return Ok(None) };
    let states: Vec<String> = raw.split(',').map(|s| s.trim().to_owned()).collect();
    match states.iter().find(|s| !STATES.contains(&s.as_str())) {
        Some(bad) => Err(ApiError::bad(format!(
            "state {bad} is not one of {}",
            STATES.join(", ")
        ))),
        None => Ok(Some(states)),
    }
}

/// An agent claim past its expiry returns the opportunity to `open`.
async fn release_expired(pool: &PgPool, workspace_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE opportunities o SET state = 'open', claim = NULL
         FROM projects p
         WHERE p.id = o.project_id AND p.workspace_id = $1 AND o.state = 'claimed'
           AND (o.claim->>'expiresAt')::timestamptz < now()",
    )
    .bind(workspace_id)
    .execute(pool)
    .await?;
    Ok(())
}

const COLUMNS: &str = "o.id, o.project_id, o.kind, o.change_kind, o.target_page_id, o.title, o.rationale,
    o.topic, o.persona, o.intent, o.score::float8 AS score, o.score::text AS score_text, o.created_at,
    o.state, o.dismissed_at, o.creation_failure, o.claim, o.evidence,
    (SELECT c.id FROM contents c WHERE c.opportunity_id = o.id ORDER BY c.created_at DESC LIMIT 1) AS content_id";

fn opportunity(r: &PgRow) -> Opportunity {
    let claim: Option<Value> = r.get("claim");
    Opportunity {
        id: r.get("id"),
        project_id: r.get("project_id"),
        kind: r.get("kind"),
        change_kind: r.get("change_kind"),
        target_page_id: r.get("target_page_id"),
        title: r.get("title"),
        rationale: r.get::<Option<String>, _>("rationale").unwrap_or_default(),
        topic: r.get::<Option<String>, _>("topic").unwrap_or_default(),
        persona: r.get::<Option<String>, _>("persona").unwrap_or_default(),
        intent: r.get::<Option<String>, _>("intent").unwrap_or_default(),
        score: r.get("score"),
        created_at: r.get("created_at"),
        state: r.get("state"),
        dismissed_at: r.get("dismissed_at"),
        creation_failure: r.get("creation_failure"),
        content_id: r.get("content_id"),
        claim: claim.and_then(|c| {
            Some(ClaimView {
                kind: c["kind"].as_str()?.to_owned(),
                claimed_at: c["claimedAt"].as_str()?.parse().ok()?,
                expires_at: c["expiresAt"].as_str().and_then(|s| s.parse().ok()),
            })
        }),
    }
}

async fn load(state: &AppState, principal: &Principal, id: Uuid) -> Result<PgRow> {
    release_expired(&state.pool, principal.workspace_id()).await?;
    sqlx::query(&format!(
        "SELECT {COLUMNS} FROM opportunities o JOIN projects p ON p.id = o.project_id
         WHERE o.id = $1 AND p.workspace_id = $2"
    ))
    .bind(id)
    .bind(principal.workspace_id())
    .fetch_optional(&state.pool)
    .await?
    .ok_or(ApiError(Error::NotFound("opportunity")))
}

fn parse_cursor(cursor: &str) -> Result<(String, Uuid)> {
    cursor
        .split_once('_')
        .and_then(|(score, id)| {
            score.parse::<f64>().ok()?;
            Some((score.to_owned(), id.parse().ok()?))
        })
        .ok_or_else(|| ApiError::bad("cursor is not valid"))
}

#[utoipa::path(
    get, path = "/v1/opportunities", tag = "Opportunities", operation_id = "list_opportunities",
    summary = "List opportunities",
    description = "The project's article opportunities and their states, highest score first. Opportunities are the article pipeline behind the Plan's `new_page` actions; work from actions, and read an opportunity to write its article from the brief.",
    security(("bearerAuth" = [])),
    params(ListQuery),
    responses(
        (status = 200, body = Opportunities),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
    )
)]
pub async fn list_opportunities(
    State(state): State<AppState>,
    principal: Principal,
    Q(q): Q<ListQuery>,
) -> Result<Json<Opportunities>> {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=100).contains(&limit) {
        return Err(ApiError::bad("limit must be 1 to 100"));
    }
    let states = parse_states(q.state.as_deref())?;
    let cursor = q.cursor.as_deref().map(parse_cursor).transpose()?;
    let project = find_project(&state, &principal, None).await?;
    release_expired(&state.pool, principal.workspace_id()).await?;
    let filter = "o.project_id = $1 AND ($2::text[] IS NULL OR o.state = ANY($2))";
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM opportunities o WHERE {filter}"
    ))
    .bind(project.id)
    .bind(&states)
    .fetch_one(&state.pool)
    .await?;
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM opportunities o WHERE {filter}
           AND ($3::text IS NULL OR (o.score, o.id) < ($3::text::numeric, $4::uuid))
         ORDER BY o.score DESC, o.id DESC LIMIT $5"
    ))
    .bind(project.id)
    .bind(&states)
    .bind(cursor.as_ref().map(|c| c.0.as_str()))
    .bind(cursor.as_ref().map(|c| c.1))
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;
    let has_next = rows.len() as i64 > limit;
    let rows = &rows[..rows.len().min(limit as usize)];
    let next_cursor = has_next.then(|| rows.last()).flatten().map(|r| {
        format!(
            "{}_{}",
            r.get::<String, _>("score_text"),
            r.get::<Uuid, _>("id")
        )
    });
    Ok(Json(Opportunities {
        opportunities: rows.iter().map(opportunity).collect(),
        page_info: PageInfo {
            limit,
            has_next_page: has_next,
            next_cursor,
            total_count: total as u64,
        },
    }))
}

#[utoipa::path(
    get, path = "/v1/opportunities/{opportunityId}", tag = "Opportunities", operation_id = "get_opportunity",
    summary = "Get an opportunity",
    description = "One opportunity with its state and the research it came from.",
    security(("bearerAuth" = [])),
    params(("opportunityId" = Uuid, Path, description = "The opportunity id")),
    responses(
        (status = 200, body = OpportunityDetail),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
    )
)]
pub async fn get_opportunity(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<OpportunityDetail>> {
    let row = load(&state, &principal, id).await?;
    let evidence: Value = row.get("evidence");
    let prompts = evidence["prompts"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|p| {
                    Some(EvidencePrompt {
                        prompt_id: p["promptId"].as_str()?.to_owned(),
                        prompt_version_id: None,
                        prompt_run_id: None,
                        platform: None,
                        evidence_kind: p["evidenceKind"].as_str()?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Json(OpportunityDetail {
        opportunity: opportunity(&row),
        prompts,
        demand_queries: vec![],
    }))
}

/// The content made from the opportunity: its id and whether it was discarded.
async fn content_of(pool: &PgPool, id: Uuid) -> Result<Option<(Uuid, bool)>> {
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, state FROM contents WHERE opportunity_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id, state)| (id, state == "discarded")))
}

fn has_content_error(content: (Uuid, bool), verb: &str) -> ApiError {
    let (id, discarded) = content;
    ApiError(Error::Unprocessable(if discarded {
        format!("content_discarded: content {id} was discarded; there is nothing left to {verb}")
    } else {
        format!(
            "has_content: this opportunity has content {id}; discard or restore the content instead"
        )
    }))
}

#[utoipa::path(
    post, path = "/v1/opportunities/{opportunityId}/dismissal", tag = "Opportunities", operation_id = "dismiss_opportunity",
    summary = "Dismiss an opportunity",
    description = "Dismiss an opportunity that has no content. Repeating it changes nothing. 422 when the opportunity has content (`has_content`, or `content_discarded` when that content is already discarded).",
    security(("bearerAuth" = [])),
    params(("opportunityId" = Uuid, Path, description = "The opportunity id")),
    responses(
        (status = 204),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody),
        (status = 422, body = ApiErrorBody),
    )
)]
pub async fn dismiss_opportunity(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    let row = load(&state, &principal, id).await?;
    if let Some(content) = content_of(&state.pool, id).await? {
        return Err(has_content_error(content, "dismiss"));
    }
    match row.get::<String, _>("state").as_str() {
        "dismissed" => {}
        "open" | "planned" | "creation_failed" => {
            sqlx::query(
                "UPDATE opportunities SET state = 'dismissed', dismissed_at = now() WHERE id = $1",
            )
            .bind(id)
            .execute(&state.pool)
            .await?;
        }
        held => {
            return Err(ApiError(Error::Conflict(format!(
                "a {held} opportunity cannot be dismissed; release it first"
            ))));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete, path = "/v1/opportunities/{opportunityId}/dismissal", tag = "Opportunities", operation_id = "restore_opportunity",
    summary = "Restore an opportunity",
    description = "Undo a dismissal on an opportunity that has no content. Repeating it changes nothing. 422 when the opportunity has content.",
    security(("bearerAuth" = [])),
    params(("opportunityId" = Uuid, Path, description = "The opportunity id")),
    responses(
        (status = 204),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 422, body = ApiErrorBody),
    )
)]
pub async fn restore_opportunity(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    let row = load(&state, &principal, id).await?;
    if let Some(content) = content_of(&state.pool, id).await? {
        return Err(has_content_error(content, "restore"));
    }
    if row.get::<String, _>("state") == "dismissed" {
        sqlx::query("UPDATE opportunities SET state = 'open', dismissed_at = NULL WHERE id = $1")
            .bind(id)
            .execute(&state.pool)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Hand the opportunity to the content worker. It leaves `open`, `planned` and `creation_failed`
/// for `writing`; the worker moves it on. Anything else is held, dismissed or already written.
pub(crate) async fn start_plan(pool: &PgPool, id: Uuid) -> Result<()> {
    let claim = json!({"kind": "pipeline", "claimedAt": Utc::now(), "expiresAt": null});
    let moved = sqlx::query(
        "UPDATE opportunities SET state = 'writing', claim = $2, creation_failure = NULL
         WHERE id = $1 AND state IN ('open', 'planned', 'creation_failed')
           AND NOT EXISTS (SELECT 1 FROM contents c WHERE c.opportunity_id = $1 AND c.state <> 'discarded')",
    )
    .bind(id)
    .bind(claim)
    .execute(pool)
    .await?
    .rows_affected();
    if moved == 0 {
        return Err(ApiError(Error::Conflict(
            "the opportunity is held, dismissed or already written".into(),
        )));
    }
    jobs::enqueue(pool, QUEUE_PLAN, &json!({"opportunity_id": id}), None, None).await?;
    Ok(())
}

#[utoipa::path(
    post, path = "/v1/opportunities/{opportunityId}/plan-requests", tag = "Opportunities", operation_id = "request_opportunity_plan",
    summary = "Have Jam write an opportunity",
    description = "Jam researches, writes and reviews content for the opportunity (Plan, or Retry after a failure). Runs in the background; the content appears under /v1/content. 409 when the opportunity is held, dismissed or already written.",
    security(("bearerAuth" = [])),
    params(("opportunityId" = Uuid, Path, description = "The opportunity id")),
    responses(
        (status = 202, body = Accepted),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody),
    )
)]
pub async fn request_opportunity_plan(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<Accepted>)> {
    load(&state, &principal, id).await?;
    start_plan(&state.pool, id).await?;
    Ok((StatusCode::ACCEPTED, Json(Accepted { status: "accepted" })))
}

fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').chars().take(80).collect()
}

fn language(locale: &str) -> Language {
    let (code, country) = match locale.split_once(['-', '_']) {
        Some((c, r)) => (c.to_ascii_lowercase(), Some(r.to_ascii_uppercase())),
        None => (locale.to_ascii_lowercase(), None),
    };
    let (name, native) = match code.as_str() {
        "en" => ("English", "English"),
        "de" => ("German", "Deutsch"),
        "fr" => ("French", "Français"),
        "es" => ("Spanish", "Español"),
        "it" => ("Italian", "Italiano"),
        "pt" => ("Portuguese", "Português"),
        "nl" => ("Dutch", "Nederlands"),
        _ => (code.as_str(), code.as_str()),
    };
    Language {
        name: name.to_owned(),
        native_name: native.to_owned(),
        code,
        country,
    }
}

#[utoipa::path(
    get, path = "/v1/opportunities/{opportunityId}/brief", tag = "Opportunities", operation_id = "get_opportunity_brief",
    summary = "Get the brief of an opportunity",
    description = "What an agent needs to write the article: the opportunity, its prompts, brand voice, sources, the article plan from research, and the exact `POST /v1/content` body.",
    security(("bearerAuth" = [])),
    params(("opportunityId" = Uuid, Path, description = "The opportunity id")),
    responses(
        (status = 200, body = OpportunityBrief),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
    )
)]
pub async fn get_opportunity_brief(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<OpportunityBrief>> {
    let row = load(&state, &principal, id).await?;
    let o = opportunity(&row);
    let evidence: Value = row.get("evidence");
    let prompt_ids: Vec<Uuid> = evidence["prompts"]
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(|p| p["promptId"].as_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    let target_prompts: Vec<String> =
        sqlx::query_scalar("SELECT text FROM prompts WHERE id = ANY($1) ORDER BY created_at")
            .bind(&prompt_ids)
            .fetch_all(&state.pool)
            .await?;
    let project = sqlx::query("SELECT name, locale FROM projects WHERE id = $1")
        .bind(o.project_id)
        .fetch_one(&state.pool)
        .await?;
    let brand = sqlx::query(
        "SELECT positioning, audience, product_description, tone, content_style, excluded_topics
         FROM brand_profiles WHERE project_id = $1 ORDER BY version DESC LIMIT 1",
    )
    .bind(o.project_id)
    .fetch_optional(&state.pool)
    .await?;
    let field = |name: &str| {
        brand
            .as_ref()
            .and_then(|b| b.get::<Option<String>, _>(name))
    };
    let plan = &evidence["plan"];
    let (article_plan, plan_unavailable) = if o.kind == "create_page" && plan.is_object() {
        let outline: Vec<Value> = plan["outline"]
            .as_array()
            .map(|l| {
                l.iter()
                    .map(|s| json!({"heading": s["heading"], "intent": s["intent"], "evidenceIds": []}))
                    .collect()
            })
            .unwrap_or_default();
        (
            Some(json!({
                "title": o.title, "intent": o.intent, "persona": o.persona,
                "angle": plan["angle"], "evidenceIds": [], "outline": outline,
            })),
            None,
        )
    } else if o.kind == "update_page" {
        (
            None,
            Some(
                "a page update has no article plan; follow the rationale and the target page"
                    .to_owned(),
            ),
        )
    } else {
        (None, Some("research stored no article plan".to_owned()))
    };
    Ok(Json(OpportunityBrief {
        opportunity_id: o.id,
        kind: o.kind.clone(),
        rationale: o.rationale.clone(),
        topic: o.topic.clone(),
        persona: o.persona.clone(),
        intent: o.intent.clone(),
        target_prompts,
        voice: Voice {
            brand: BrandVoice {
                name: project.get("name"),
                positioning: field("positioning"),
                audience: field("audience"),
                product_description: field("product_description"),
                tone: field("tone"),
                content_style: field("content_style"),
                excluded_topics: brand
                    .as_ref()
                    .map(|b| b.get("excluded_topics"))
                    .unwrap_or_default(),
            },
            language: language(&project.get::<String, _>("locale")),
        },
        research: BriefResearch {
            evidence_bundle_id: o.id.to_string(),
            sources: evidence["sources"].clone(),
        },
        article_plan,
        credits_charged: 0,
        plan_unavailable,
        delivery: None,
        submit: Submit {
            method: "POST",
            path: "/v1/content",
            body: json!({
                "opportunityId": o.id,
                "bundleId": null,
                "article": {
                    "title": o.title, "metaDescription": "", "slug": slug(&o.title),
                    "keywords": [], "bodyMarkdown": "",
                },
            }),
        },
        title: o.title,
    }))
}

#[utoipa::path(
    post, path = "/v1/opportunities/{opportunityId}/claims", tag = "Opportunities", operation_id = "claim_opportunity",
    summary = "Claim an opportunity for an agent",
    description = "Hold the opportunity while your agent writes it: it becomes `claimed`, and neither Jam nor another agent writes it. Released by `POST /v1/content`, `DELETE .../claims/current`, `POST .../agent-failures`, or after 24 hours. 409 when it is already held, planned or dismissed.",
    security(("bearerAuth" = [])),
    params(("opportunityId" = Uuid, Path, description = "The opportunity id")),
    request_body = Claim,
    responses(
        (status = 201, body = ClaimOpened),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody),
    )
)]
pub async fn claim_opportunity(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<ClaimOpened>)> {
    load(&state, &principal, id).await?;
    let claim_id = Uuid::new_v4();
    let now = Utc::now();
    let expires_at = now + Duration::hours(CLAIM_HOURS);
    let claim =
        json!({"kind": "agent", "claimId": claim_id, "claimedAt": now, "expiresAt": expires_at});
    let moved = sqlx::query(
        "UPDATE opportunities SET state = 'claimed', claim = $2
         WHERE id = $1 AND state IN ('open', 'creation_failed')
           AND NOT EXISTS (SELECT 1 FROM contents c WHERE c.opportunity_id = $1 AND c.state <> 'discarded')",
    )
    .bind(id)
    .bind(claim)
    .execute(&state.pool)
    .await?
    .rows_affected();
    if moved == 0 {
        return Err(ApiError(Error::Conflict(
            "the opportunity is already held, planned or dismissed".into(),
        )));
    }
    Ok((
        StatusCode::CREATED,
        Json(ClaimOpened {
            claim_id,
            expires_at,
        }),
    ))
}

#[utoipa::path(
    delete, path = "/v1/opportunities/{opportunityId}/claims/current", tag = "Opportunities", operation_id = "release_opportunity_claim",
    summary = "Release an agent claim",
    description = "Give the opportunity back without a result; it is open again. 404 when no agent claim is open.",
    security(("bearerAuth" = [])),
    params(("opportunityId" = Uuid, Path, description = "The opportunity id")),
    responses(
        (status = 204),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
    )
)]
pub async fn release_opportunity_claim(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    load(&state, &principal, id).await?;
    let moved = sqlx::query(
        "UPDATE opportunities SET state = 'open', claim = NULL
         WHERE id = $1 AND state = 'claimed' AND claim->>'kind' = 'agent'",
    )
    .bind(id)
    .execute(&state.pool)
    .await?
    .rows_affected();
    if moved == 0 {
        return Err(ApiError(Error::NotFound("agent claim")));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post, path = "/v1/opportunities/{opportunityId}/agent-failures", tag = "Opportunities", operation_id = "report_opportunity_agent_failure",
    summary = "Report that the agent failed",
    description = "Release the agent claim and record `reason` as the failure users see (`creationFailure.code` `agent_failed`). 409 when the opportunity already has content.",
    security(("bearerAuth" = [])),
    params(("opportunityId" = Uuid, Path, description = "The opportunity id")),
    request_body = AgentFailure,
    responses(
        (status = 204),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody),
    )
)]
pub async fn report_opportunity_agent_failure(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
    Json(body): Json<AgentFailure>,
) -> Result<StatusCode> {
    let reason: String = body.reason.trim().chars().take(1000).collect();
    if reason.is_empty() {
        return Err(ApiError::bad(
            "reason says why the article could not be written",
        ));
    }
    load(&state, &principal, id).await?;
    if content_of(&state.pool, id).await?.is_some() {
        return Err(ApiError(Error::Conflict(
            "the opportunity already has content".into(),
        )));
    }
    let failure = json!({"code": "agent_failed", "message": reason, "at": Utc::now()});
    sqlx::query(
        "UPDATE opportunities SET state = 'creation_failed', claim = NULL, creation_failure = $2
         WHERE id = $1 AND state IN ('claimed', 'open', 'creation_failed')",
    )
    .bind(id)
    .bind(failure)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
