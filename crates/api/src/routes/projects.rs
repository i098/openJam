use crate::error::{ApiError, Result};
use crate::{auth::Principal, error::ApiErrorBody, state::AppState, types::*};
use axum::{Json, extract::State};

/// The workspace's projects that are not archived, newest first.
#[utoipa::path(
    get, path = "/v1/projects", tag = "Projects", operation_id = "list_projects",
    summary = "List projects", security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ProjectList),
        (status = 401, body = ApiErrorBody)
    )
)]
pub async fn list_projects(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<ProjectList>> {
    let projects: Vec<ProjectSummary> = sqlx::query_as(
        "SELECT id, name, domain, site_url, status, created_at FROM projects
         WHERE workspace_id = $1 AND archived_at IS NULL ORDER BY created_at DESC",
    )
    .bind(principal.workspace_id())
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(ProjectList { projects }))
}

#[utoipa::path(
    post, path = "/v1/projects", tag = "Projects", operation_id = "create_project",
    summary = "Create a project", security(("bearerAuth" = [])), request_body = CreateProject,
    responses(
        (status = 201, body = ProjectSummary),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody)
    )
)]
pub async fn create_project(
    State(state): State<AppState>,
    principal: Principal,
    Json(body): Json<CreateProject>,
) -> Result<(axum::http::StatusCode, Json<ProjectSummary>)> {
    let name = body.name.trim().to_string();
    if name.is_empty() || name.len() > 80 {
        return Err(ApiError::bad("name must be 1 to 80 characters"));
    }
    let domain = body
        .domain
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_lowercase();
    let valid = domain.len() <= 253
        && domain.contains('.')
        && !domain.contains(char::is_whitespace)
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '/'));
    if !valid {
        return Err(ApiError::bad(
            "domain must be a host name, for example acme.com",
        ));
    }
    let site_url = body.site_url.unwrap_or_else(|| format!("https://{domain}"));
    let project: ProjectSummary = sqlx::query_as(
        "INSERT INTO projects (workspace_id, name, domain, site_url) VALUES ($1, $2, $3, $4)
         RETURNING id, name, domain, site_url, status, created_at",
    )
    .bind(principal.workspace_id())
    .bind(&name)
    .bind(&domain)
    .bind(&site_url)
    .fetch_one(&state.pool)
    .await?;
    Ok((axum::http::StatusCode::CREATED, Json(project)))
}
