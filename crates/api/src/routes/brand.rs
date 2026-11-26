//! `/v1/brand`: the brand profile versions and the pinned competitors.

use crate::analytics::find_project;
use crate::auth::Principal;
use crate::error::{ApiError, ApiErrorBody, Result};
use crate::state::AppState;
use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrandCompetitor {
    pub name: String,
    pub domain: Option<String>,
    pub aliases: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrandProfile {
    /// 0 when the brand profile has no saved version yet.
    pub version: i32,
    pub positioning: Option<String>,
    pub audience: Option<String>,
    pub product_description: Option<String>,
    /// The competitor names, in the order they were added.
    pub competitors: Vec<String>,
    pub competitor_roster: Vec<BrandCompetitor>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBrand {
    pub positioning: Option<String>,
    pub audience: Option<String>,
    pub product_description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddCompetitors {
    pub competitors: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CompetitorList {
    pub competitors: Vec<BrandCompetitorRow>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrandCompetitorRow {
    pub name: String,
    pub domain: Option<String>,
    pub aliases: Vec<String>,
    pub pinned_at: Option<DateTime<Utc>>,
}

async fn roster(state: &AppState, project_id: Uuid) -> Result<Vec<BrandCompetitor>> {
    let rows: Vec<BrandCompetitorRow> = sqlx::query_as(
        "SELECT name, domain, aliases, pinned_at FROM competitors
         WHERE project_id = $1 ORDER BY pinned_at NULLS LAST, name",
    )
    .bind(project_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BrandCompetitor {
            name: r.name,
            domain: r.domain,
            aliases: r.aliases,
        })
        .collect())
}

#[utoipa::path(
    get, path = "/v1/brand", tag = "Brand", operation_id = "get_brand",
    summary = "Get the brand profile", security(("bearerAuth" = [])),
    params(("projectId" = Option<Uuid>, Query, description = "The project; newest when omitted")),
    responses((status = 200, body = BrandProfile), (status = 401, body = ApiErrorBody))
)]
pub async fn get_brand(
    State(state): State<AppState>,
    principal: Principal,
    axum::extract::Query(query): axum::extract::Query<BrandQuery>,
) -> Result<Json<BrandProfile>> {
    let project = find_project(&state, &principal, query.project_id).await?;
    type Row = (i32, Option<String>, Option<String>, Option<String>);
    let row: Option<Row> = sqlx::query_as(
        "SELECT version, positioning, audience, product_description FROM brand_profiles
         WHERE project_id = $1 ORDER BY version DESC LIMIT 1",
    )
    .bind(project.id)
    .fetch_optional(&state.pool)
    .await?;
    let roster = roster(&state, project.id).await?;
    let (version, positioning, audience, product_description) =
        row.unwrap_or((0, None, None, None));
    Ok(Json(BrandProfile {
        version,
        positioning,
        audience,
        product_description,
        competitors: roster.iter().map(|c| c.name.clone()).collect(),
        competitor_roster: roster,
    }))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct BrandQuery {
    pub project_id: Option<Uuid>,
}

/// Save a new brand profile version. Fields left out keep their last value.
#[utoipa::path(
    patch, path = "/v1/brand", tag = "Brand", operation_id = "update_brand",
    summary = "Update the brand profile", security(("bearerAuth" = [])),
    params(("projectId" = Option<Uuid>, Query, description = "The project; newest when omitted")),
    request_body = UpdateBrand,
    responses(
        (status = 200, body = BrandProfile),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody)
    )
)]
pub async fn update_brand(
    State(state): State<AppState>,
    principal: Principal,
    axum::extract::Query(query): axum::extract::Query<BrandQuery>,
    Json(body): Json<UpdateBrand>,
) -> Result<Json<BrandProfile>> {
    let project = find_project(&state, &principal, query.project_id).await?;
    for value in [&body.positioning, &body.audience, &body.product_description]
        .into_iter()
        .flatten()
    {
        if value.chars().count() > 2000 {
            return Err(ApiError::bad(
                "a brand field is longer than 2000 characters",
            ));
        }
    }
    sqlx::query(
        "INSERT INTO brand_profiles
             (project_id, version, positioning, audience, product_description)
         VALUES ($1,
                COALESCE((SELECT MAX(version) FROM brand_profiles WHERE project_id = $1), 0) + 1,
                COALESCE($2, (SELECT positioning FROM brand_profiles
                              WHERE project_id = $1 ORDER BY version DESC LIMIT 1)),
                COALESCE($3, (SELECT audience FROM brand_profiles
                              WHERE project_id = $1 ORDER BY version DESC LIMIT 1)),
                COALESCE($4, (SELECT product_description FROM brand_profiles
                              WHERE project_id = $1 ORDER BY version DESC LIMIT 1)))",
    )
    .bind(project.id)
    .bind(
        body.positioning
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
    )
    .bind(
        body.audience
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
    )
    .bind(
        body.product_description
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
    )
    .execute(&state.pool)
    .await?;
    let profile = get_brand(
        State(state),
        principal,
        axum::extract::Query(BrandQuery {
            project_id: Some(project.id),
        }),
    )
    .await?;
    Ok(profile)
}

#[utoipa::path(
    get, path = "/v1/brand/competitors", tag = "Brand", operation_id = "list_brand_competitors",
    summary = "List the pinned competitors", security(("bearerAuth" = [])),
    params(("projectId" = Option<Uuid>, Query, description = "The project; newest when omitted")),
    responses((status = 200, body = CompetitorList), (status = 401, body = ApiErrorBody))
)]
pub async fn list_brand_competitors(
    State(state): State<AppState>,
    principal: Principal,
    axum::extract::Query(query): axum::extract::Query<BrandQuery>,
) -> Result<Json<CompetitorList>> {
    let project = find_project(&state, &principal, query.project_id).await?;
    let competitors: Vec<BrandCompetitorRow> = sqlx::query_as(
        "SELECT name, domain, aliases, pinned_at FROM competitors
         WHERE project_id = $1 ORDER BY pinned_at NULLS LAST, name",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(CompetitorList { competitors }))
}

/// Pin competitors. Adding a name that is pinned already keeps its row.
#[utoipa::path(
    post, path = "/v1/brand/competitors", tag = "Brand", operation_id = "add_brand_competitors",
    summary = "Add competitors", security(("bearerAuth" = [])),
    params(("projectId" = Option<Uuid>, Query, description = "The project; newest when omitted")),
    request_body = AddCompetitors,
    responses(
        (status = 200, body = CompetitorList),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody)
    )
)]
pub async fn add_brand_competitors(
    State(state): State<AppState>,
    principal: Principal,
    axum::extract::Query(query): axum::extract::Query<BrandQuery>,
    Json(body): Json<AddCompetitors>,
) -> Result<Json<CompetitorList>> {
    let project = find_project(&state, &principal, query.project_id).await?;
    let mut names = Vec::new();
    for raw in &body.competitors {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        if name.chars().count() > 120 {
            return Err(ApiError::bad(
                "a competitor name is longer than 120 characters",
            ));
        }
        names.push(name.to_owned());
    }
    if names.is_empty() {
        return Err(ApiError::bad("competitors must hold at least one name"));
    }
    let mut tx = state.pool.begin().await?;
    for name in &names {
        sqlx::query(
            "INSERT INTO competitors (project_id, name, pinned_at) VALUES ($1, $2, now())
             ON CONFLICT (project_id, name) DO UPDATE
             SET pinned_at = COALESCE(competitors.pinned_at, now())",
        )
        .bind(project.id)
        .bind(name)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    list_brand_competitors(
        State(state),
        principal,
        axum::extract::Query(BrandQuery {
            project_id: Some(project.id),
        }),
    )
    .await
}
