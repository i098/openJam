//! Destinations, the GitHub connection, and the publishing of content: deliveries and publications.

use crate::content::{active_project, require_content};
use crate::error::{ApiError, ApiErrorBody, Error, Result};
use crate::{analytics::Q, auth::Principal, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use openjam_publish::{
    article::{REQUIRED_FRONTMATTER, is_safe_folder},
    delivery::{self, GithubConfig},
    github::GithubError,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct Connection {
    /// `ok` or `problem`.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Serialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Destination {
    pub id: Uuid,
    pub project_id: Uuid,
    /// Always `github`.
    pub platform: String,
    pub auto_publish: bool,
    /// `daily` or `weekly`.
    pub cadence: String,
    pub created_at: DateTime<Utc>,
    /// Whether the platform has a human review step; auto-publish requires one.
    #[sqlx(skip)]
    pub review_surface: bool,
    pub display_name: String,
    #[sqlx(skip)]
    pub connection: Option<Connection>,
    #[serde(skip)]
    connection_json: Value,
}

impl Destination {
    fn finish(mut self) -> Self {
        self.review_surface = true;
        self.connection = Some(Connection {
            status: self.connection_json["status"]
                .as_str()
                .unwrap_or("problem")
                .into(),
            message: self.connection_json["message"].as_str().map(str::to_owned),
        });
        self
    }
}

const SELECT_DESTINATION: &str = "SELECT d.id, d.project_id, d.platform, d.auto_publish, d.cadence,
        d.created_at, COALESCE(d.display_name, 'GitHub') AS display_name,
        d.connection AS connection_json
    FROM destinations d JOIN projects p ON p.id = d.project_id";

#[derive(Debug, Serialize, ToSchema)]
pub struct Destinations {
    pub destinations: Vec<Destination>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddDestination {
    /// Only `github`.
    pub platform: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddedDestination {
    pub destination_id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DestinationPolicy {
    /// Publish drafted content automatically on the cadence. Turning it on turns it off on the
    /// project's other destinations.
    pub auto_publish: Option<bool>,
    /// `daily` or `weekly`.
    pub cadence: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfigureGithubDestination {
    pub owner: String,
    pub name: String,
    pub base_branch: String,
    /// The folder articles live in.
    pub content_root: String,
    /// The address an article is published at, with `{slug}` where its slug goes.
    pub public_route_pattern: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubDestination {
    pub destination_id: Uuid,
    pub repository: String,
    pub base_branch: String,
    pub content_root: String,
    pub public_route_pattern: String,
    pub file_extension: String,
    pub required_frontmatter: Vec<&'static str>,
}

async fn load_destination(pool: &PgPool, principal: &Principal, id: Uuid) -> Result<Destination> {
    let row: Destination = sqlx::query_as(&format!(
        "{SELECT_DESTINATION} WHERE d.id = $1 AND p.workspace_id = $2"
    ))
    .bind(id)
    .bind(principal.workspace_id())
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError(Error::NotFound("destination")))?;
    Ok(row.finish())
}

/// The active project's destinations with their policy and connection status.
#[utoipa::path(
    get, path = "/v1/destinations", tag = "Destinations", operation_id = "list_destinations",
    summary = "List destinations", security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Destinations),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn list_destinations(
    State(state): State<AppState>,
    principal: Principal,
    headers: HeaderMap,
) -> Result<Json<Destinations>> {
    let project = active_project(&state.pool, &principal, &headers).await?;
    let rows: Vec<Destination> = sqlx::query_as(&format!(
        "{SELECT_DESTINATION} WHERE d.project_id = $1 ORDER BY d.created_at"
    ))
    .bind(project)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(Destinations {
        destinations: rows.into_iter().map(Destination::finish).collect(),
    }))
}

/// Add a destination to the active project.
#[utoipa::path(
    post, path = "/v1/destinations", tag = "Destinations", operation_id = "create_destination",
    summary = "Add a destination", security(("bearerAuth" = [])), request_body = AddDestination,
    responses(
        (status = 201, body = AddedDestination),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn create_destination(
    State(state): State<AppState>,
    principal: Principal,
    headers: HeaderMap,
    Json(body): Json<AddDestination>,
) -> Result<(StatusCode, Json<AddedDestination>)> {
    if body.platform != "github" {
        return Err(ApiError::bad("platform must be github"));
    }
    let project = active_project(&state.pool, &principal, &headers).await?;
    let destination_id = sqlx::query_scalar(
        "INSERT INTO destinations (project_id, display_name) VALUES ($1, 'GitHub') RETURNING id",
    )
    .bind(project)
    .fetch_one(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(AddedDestination { destination_id }),
    ))
}

/// Change a destination's auto-publish or cadence.
#[utoipa::path(
    patch, path = "/v1/destinations/{destinationId}", tag = "Destinations", operation_id = "update_destination",
    summary = "Change publishing policy", security(("bearerAuth" = [])), request_body = DestinationPolicy,
    params(("destinationId" = Uuid, Path, description = "The destination")),
    responses(
        (status = 204, description = "The policy is changed."),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn update_destination(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
    Json(body): Json<DestinationPolicy>,
) -> Result<StatusCode> {
    let destination = load_destination(&state.pool, &principal, id).await?;
    if let Some(cadence) = &body.cadence
        && cadence != "daily"
        && cadence != "weekly"
    {
        return Err(ApiError::bad("cadence must be daily or weekly"));
    }
    let mut tx = state.pool.begin().await?;
    match body.auto_publish {
        Some(true) => {
            let connected = destination
                .connection
                .as_ref()
                .is_some_and(|c| c.status == "ok");
            if !connected {
                return Err(ApiError::bad(
                    "connect a repository first: auto-publish needs a working destination",
                ));
            }
            let others: Vec<Uuid> = sqlx::query_scalar(
                "UPDATE destinations SET auto_publish = false
                 WHERE project_id = $1 AND id <> $2 AND auto_publish RETURNING id",
            )
            .bind(destination.project_id)
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
            for other in others {
                sqlx::query(
                    "UPDATE deliveries SET status = 'cancelled', cancelled_at = now()
                     WHERE destination_id = $1 AND status = 'scheduled' AND automatic",
                )
                .bind(other)
                .execute(&mut *tx)
                .await?;
            }
            // The next worker tick publishes a draft; the cadence counts from then on.
            sqlx::query(
                "UPDATE destinations SET auto_publish = true, last_auto_at = NULL WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        Some(false) => {
            sqlx::query("UPDATE destinations SET auto_publish = false WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE deliveries SET status = 'cancelled', cancelled_at = now()
                 WHERE destination_id = $1 AND status = 'scheduled' AND automatic",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        None => {}
    }
    if let Some(cadence) = &body.cadence {
        sqlx::query("UPDATE destinations SET cadence = $2 WHERE id = $1")
            .bind(id)
            .bind(cadence)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Remove a destination and its open schedules. Repeating it changes nothing.
#[utoipa::path(
    delete, path = "/v1/destinations/{destinationId}", tag = "Destinations", operation_id = "remove_destination",
    summary = "Remove a destination", security(("bearerAuth" = [])),
    params(("destinationId" = Uuid, Path, description = "The destination")),
    responses(
        (status = 204, description = "The destination is removed."),
        (status = 401, body = ApiErrorBody)
    )
)]
pub async fn remove_destination(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    sqlx::query(
        "DELETE FROM destinations d USING projects p
         WHERE d.id = $1 AND p.id = d.project_id AND p.workspace_id = $2",
    )
    .bind(id)
    .bind(principal.workspace_id())
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn github_problem(e: GithubError) -> ApiError {
    if e.is_permanent() {
        ApiError::bad(e.to_string())
    } else {
        ApiError(Error::Internal(e.to_string()))
    }
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// Connect a destination to a repository the GitHub App is installed on.
#[utoipa::path(
    put, path = "/v1/destinations/{destinationId}/github", tag = "Destinations", operation_id = "configure_destination_github",
    summary = "Connect a GitHub repository", security(("bearerAuth" = [])), request_body = ConfigureGithubDestination,
    params(("destinationId" = Uuid, Path, description = "The destination")),
    responses(
        (status = 200, body = GithubDestination),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn configure_destination_github(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
    Json(body): Json<ConfigureGithubDestination>,
) -> Result<Json<GithubDestination>> {
    load_destination(&state.pool, &principal, id).await?;
    let (owner, name) = (body.owner.trim(), body.name.trim());
    let base = body.base_branch.trim();
    let root = body.content_root.trim();
    let pattern = body.public_route_pattern.trim();
    if !valid_name(owner) || !valid_name(name) {
        return Err(ApiError::bad(
            "owner and name must be a GitHub owner and repository name",
        ));
    }
    if base.is_empty() || !is_safe_folder(base) || base.starts_with('/') {
        return Err(ApiError::bad("baseBranch is not a valid branch name"));
    }
    if !is_safe_folder(root) {
        return Err(ApiError::bad(
            "contentRoot must be a folder inside the repository",
        ));
    }
    if !pattern.contains("{slug}") || !(pattern.starts_with('/') || pattern.starts_with("http")) {
        return Err(ApiError::bad(
            "publicRoutePattern must start with / or http and contain {slug}",
        ));
    }

    let app = state.github.as_deref().ok_or_else(|| {
        ApiError::bad(
            "GitHub publishing is off: set OPENJAM_GITHUB_APP_ID and the app's private key",
        )
    })?;
    let installation = app
        .installation(owner, name)
        .await
        .map_err(github_problem)?
        .ok_or_else(|| {
            ApiError::bad(format!(
                "install the GitHub App on {owner}/{name}, then try again"
            ))
        })?;
    if !installation.can_open_prs {
        return Err(ApiError::bad(
            "the GitHub App needs read and write access to Contents and Pull requests",
        ));
    }
    let repo = app
        .repo(installation.id, owner, name)
        .await
        .map_err(github_problem)?;
    if repo
        .branch_sha(base)
        .await
        .map_err(github_problem)?
        .is_none()
    {
        return Err(ApiError::bad(format!(
            "the branch {base} does not exist in {owner}/{name}"
        )));
    }
    let files = repo
        .list_dir(base, root.trim_matches('/'))
        .await
        .map_err(github_problem)?;
    let count = |ext: &str| files.iter().filter(|f| f.ends_with(ext)).count();
    let file_extension = if count(".mdx") > count(".md") {
        "mdx"
    } else {
        "md"
    };

    let config = GithubConfig {
        owner: owner.into(),
        name: name.into(),
        base_branch: base.into(),
        content_root: root.trim_matches('/').into(),
        public_route_pattern: pattern.into(),
        file_extension: file_extension.into(),
        installation_id: installation.id,
    };
    sqlx::query(
        "UPDATE destinations SET config = $2, display_name = $3, connection = $4 WHERE id = $1",
    )
    .bind(id)
    .bind(json!(config))
    .bind(format!("{owner}/{name}"))
    .bind(json!({"status": "ok"}))
    .execute(&state.pool)
    .await?;
    Ok(Json(GithubDestination {
        destination_id: id,
        repository: format!("{owner}/{name}"),
        base_branch: config.base_branch,
        content_root: config.content_root,
        public_route_pattern: config.public_route_pattern,
        file_extension: config.file_extension,
        required_frontmatter: REQUIRED_FRONTMATTER.to_vec(),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContentDestination {
    pub destination_id: Uuid,
    pub platform: String,
    /// `scheduled`, `in_review`, `live`, `failed` or `cancelled`.
    pub state: String,
    /// The latest delivery's time.
    pub publish_at: DateTime<Utc>,
    pub latest_delivery_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContentDestinations {
    pub destinations: Vec<ContentDestination>,
}

/// Each destination the content is scheduled for, with the state of its latest delivery.
#[utoipa::path(
    get, path = "/v1/content/{contentId}/destinations", tag = "Publishing", operation_id = "list_content_destinations",
    summary = "Content's destinations", security(("bearerAuth" = [])),
    params(("contentId" = Uuid, Path, description = "The content item")),
    responses(
        (status = 200, body = ContentDestinations),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn list_content_destinations(
    State(state): State<AppState>,
    principal: Principal,
    Path(content_id): Path<Uuid>,
) -> Result<Json<ContentDestinations>> {
    require_content(&state.pool, &principal, content_id).await?;
    let rows: Vec<(Uuid, String, String, DateTime<Utc>, Uuid)> = sqlx::query_as(
        "SELECT DISTINCT ON (d.destination_id) d.destination_id, dest.platform, d.status, d.deliver_at, d.id
         FROM deliveries d JOIN destinations dest ON dest.id = d.destination_id
         WHERE d.content_id = $1
         ORDER BY d.destination_id, d.created_at DESC",
    )
    .bind(content_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(ContentDestinations {
        destinations: rows
            .into_iter()
            .map(
                |(destination_id, platform, state, publish_at, latest_delivery_id)| {
                    ContentDestination {
                        destination_id,
                        platform,
                        state,
                        publish_at,
                        latest_delivery_id,
                    }
                },
            )
            .collect(),
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Publish {
    /// Send `"now"` to publish now. Send this or `publishAt`, not both.
    pub when: Option<String>,
    /// When to publish, as an ISO-8601 timestamp.
    pub publish_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedDelivery {
    /// Always `accepted`.
    pub status: &'static str,
    /// The delivery to follow under `/v1/content/{contentId}/deliveries`.
    pub delivery_id: Uuid,
}

/// Ship the current revision to a destination now or at a time.
#[utoipa::path(
    put, path = "/v1/content/{contentId}/destinations/{destinationId}", tag = "Publishing", operation_id = "publish_content",
    summary = "Publish content to a destination", security(("bearerAuth" = [])), request_body = Publish,
    params(
        ("contentId" = Uuid, Path, description = "The content item"),
        ("destinationId" = Uuid, Path, description = "The destination")
    ),
    responses(
        (status = 202, body = AcceptedDelivery),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody)
    )
)]
pub async fn publish_content(
    State(state): State<AppState>,
    principal: Principal,
    Path((content_id, destination_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<Publish>,
) -> Result<(StatusCode, Json<AcceptedDelivery>)> {
    let deliver_at = match (body.when.as_deref(), body.publish_at) {
        (Some("now"), None) => Utc::now(),
        (None, Some(at)) => at,
        _ => {
            return Err(ApiError::bad(
                "send either when: \"now\" or publishAt, not both",
            ));
        }
    };
    require_content(&state.pool, &principal, content_id).await?;
    let destination = load_destination(&state.pool, &principal, destination_id).await?;
    let (content_project, content_state): (Uuid, String) =
        sqlx::query_as("SELECT project_id, state FROM contents WHERE id = $1")
            .bind(content_id)
            .fetch_one(&state.pool)
            .await?;
    if content_project != destination.project_id {
        return Err(ApiError(Error::NotFound("destination")));
    }
    if content_state != "drafted" {
        return Err(ApiError(Error::Conflict(
            "the content is discarded; restore it first".into(),
        )));
    }
    let configured: bool =
        sqlx::query_scalar("SELECT config ? 'owner' FROM destinations WHERE id = $1")
            .bind(destination_id)
            .fetch_one(&state.pool)
            .await?;
    if !configured {
        return Err(ApiError::bad(
            "the destination has no repository: PUT /v1/destinations/{destinationId}/github first",
        ));
    }
    let delivery_id =
        delivery::schedule(&state.pool, content_id, destination_id, deliver_at, false).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(AcceptedDelivery {
            status: "accepted",
            delivery_id,
        }),
    ))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CancelQuery {
    /// `true` confirms that cancelling may close the pull request that is open.
    pub withdraw: Option<String>,
}

/// Cancel the latest schedule of a content item at a destination.
#[utoipa::path(
    delete, path = "/v1/content/{contentId}/destinations/{destinationId}", tag = "Publishing", operation_id = "cancel_content_publishing",
    summary = "Cancel publishing to a destination", security(("bearerAuth" = [])),
    params(
        ("contentId" = Uuid, Path, description = "The content item"),
        ("destinationId" = Uuid, Path, description = "The destination"),
        CancelQuery
    ),
    responses(
        (status = 204, description = "The schedule is cancelled."),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody)
    )
)]
pub async fn cancel_content_publishing(
    State(state): State<AppState>,
    principal: Principal,
    Path((content_id, destination_id)): Path<(Uuid, Uuid)>,
    Q(q): Q<CancelQuery>,
) -> Result<StatusCode> {
    require_content(&state.pool, &principal, content_id).await?;
    load_destination(&state.pool, &principal, destination_id).await?;
    let withdraw = match q.withdraw.as_deref() {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => return Err(ApiError::bad("withdraw must be true or false")),
    };
    delivery::cancel_latest(&state.pool, content_id, destination_id, withdraw).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub id: Uuid,
    pub destination_id: Uuid,
    /// Always `github`.
    pub platform: String,
    /// Created by auto-publish rather than a person.
    pub automatic: bool,
    /// When it was due to go out.
    pub deliver_at: DateTime<Utc>,
    pub cancelled_at: Option<DateTime<Utc>>,
    /// Null while it waits.
    pub attempted_at: Option<DateTime<Utc>>,
    pub failure_message: Option<String>,
    /// The delivered item's page on the platform: the pull request.
    pub item_url: Option<String>,
    pub publication_id: Option<Uuid>,
    /// `scheduled`, `in_review`, `live`, `failed` or `cancelled`.
    pub state: String,
    pub pr_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Deliveries {
    pub deliveries: Vec<Delivery>,
}

/// Every request to send the content to a destination, newest first.
#[utoipa::path(
    get, path = "/v1/content/{contentId}/deliveries", tag = "Deliveries", operation_id = "list_content_deliveries",
    summary = "Content's deliveries", security(("bearerAuth" = [])),
    params(("contentId" = Uuid, Path, description = "The content item")),
    responses(
        (status = 200, body = Deliveries),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn list_content_deliveries(
    State(state): State<AppState>,
    principal: Principal,
    Path(content_id): Path<Uuid>,
) -> Result<Json<Deliveries>> {
    require_content(&state.pool, &principal, content_id).await?;
    let deliveries = sqlx::query_as(
        "SELECT d.id, d.destination_id, dest.platform, d.automatic, d.deliver_at, d.cancelled_at,
                d.attempted_at, d.error AS failure_message, d.pr_url AS item_url, d.publication_id,
                d.status AS state, d.pr_url, d.created_at
         FROM deliveries d JOIN destinations dest ON dest.id = d.destination_id
         WHERE d.content_id = $1 ORDER BY d.created_at DESC, d.id DESC",
    )
    .bind(content_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(Deliveries { deliveries }))
}

#[derive(Debug, Serialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Publication {
    pub id: Uuid,
    pub content_id: Uuid,
    /// The public page the content is live at.
    pub url: String,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Publications {
    pub publications: Vec<Publication>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MarkPublished {
    /// The public page the content is live at; http or https.
    pub url: String,
}

/// Every page the content is live at, newest first.
#[utoipa::path(
    get, path = "/v1/content/{contentId}/publications", tag = "Publications", operation_id = "list_content_publications",
    summary = "Content's publications", security(("bearerAuth" = [])),
    params(("contentId" = Uuid, Path, description = "The content item")),
    responses(
        (status = 200, body = Publications),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn list_content_publications(
    State(state): State<AppState>,
    principal: Principal,
    Path(content_id): Path<Uuid>,
) -> Result<Json<Publications>> {
    require_content(&state.pool, &principal, content_id).await?;
    let publications = sqlx::query_as(
        "SELECT id, content_id, url, published_at FROM publications
         WHERE content_id = $1 ORDER BY published_at DESC, id DESC",
    )
    .bind(content_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(Publications { publications }))
}

/// Record that the content was published by hand at a URL. The same URL again returns the
/// existing publication.
#[utoipa::path(
    post, path = "/v1/content/{contentId}/publications", tag = "Publications", operation_id = "mark_content_published",
    summary = "Mark as published", security(("bearerAuth" = [])), request_body = MarkPublished,
    params(("contentId" = Uuid, Path, description = "The content item")),
    responses(
        (status = 201, body = Publication),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody),
        (status = 409, body = ApiErrorBody)
    )
)]
pub async fn mark_content_published(
    State(state): State<AppState>,
    principal: Principal,
    Path(content_id): Path<Uuid>,
    Json(body): Json<MarkPublished>,
) -> Result<(StatusCode, Json<Publication>)> {
    let url = body.url.trim();
    let well_formed = (url.starts_with("https://") || url.starts_with("http://"))
        && url.len() <= 2048
        && !url.contains(char::is_whitespace)
        && url.split("://").nth(1).is_some_and(|rest| !rest.is_empty());
    if !well_formed {
        return Err(ApiError::bad("url must be an http or https address"));
    }
    require_content(&state.pool, &principal, content_id).await?;
    let discarded: bool =
        sqlx::query_scalar("SELECT state = 'discarded' FROM contents WHERE id = $1")
            .bind(content_id)
            .fetch_one(&state.pool)
            .await?;
    if discarded {
        return Err(ApiError(Error::Conflict(
            "the content is discarded; restore it first".into(),
        )));
    }
    let publication = sqlx::query_as(
        "INSERT INTO publications (content_id, url) VALUES ($1, $2)
         ON CONFLICT (content_id, url) DO UPDATE SET url = EXCLUDED.url
         RETURNING id, content_id, url, published_at",
    )
    .bind(content_id)
    .bind(url)
    .fetch_one(&state.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(publication)))
}
