use crate::error::{ApiError, Error, Result};
use crate::{
    auth::{Principal, Role},
    error::ApiErrorBody,
    state::AppState,
    types::*,
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use openjam_core::auth::new_api_key;
use uuid::Uuid;

/// Every key in the workspace. The secret is not returned again.
#[utoipa::path(
    get, path = "/v1/api-keys", tag = "API keys", operation_id = "list_api_keys",
    summary = "List API keys", security(("bearerAuth" = [])),
    responses((status = 200, body = ApiKeyList), (status = 401, body = ApiErrorBody))
)]
pub async fn list_api_keys(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<ApiKeyList>> {
    let keys: Vec<ApiKeySummary> = sqlx::query_as(
        "SELECT id, name, prefix AS key_prefix, created_at, last_used_at, revoked_at
         FROM api_keys WHERE workspace_id = $1 ORDER BY created_at DESC",
    )
    .bind(principal.workspace_id())
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(ApiKeyList { keys }))
}

/// Issue a key. The secret is in this response only. An API key cannot create keys.
#[utoipa::path(
    post, path = "/v1/api-keys", tag = "API keys", operation_id = "create_api_key",
    summary = "Create an API key", security(("bearerAuth" = [])), request_body = IssueApiKey,
    responses(
        (status = 201, body = IssuedApiKey),
        (status = 401, body = ApiErrorBody),
        (status = 403, body = ApiErrorBody)
    )
)]
pub async fn create_api_key(
    State(state): State<AppState>,
    principal: Principal,
    Json(body): Json<IssueApiKey>,
) -> Result<(StatusCode, Json<IssuedApiKey>)> {
    let owner = match &principal {
        Principal::Key(_) => return Err(ApiError(Error::ForbiddenForApiKey)),
        Principal::Session(s) if s.role == Role::Owner => true,
        Principal::Session(_) => false,
    };
    if !owner {
        return Err(ApiError(Error::Forbidden));
    }
    let name = body.name.trim().to_string();
    if name.is_empty() || name.len() > 80 {
        return Err(ApiError::bad("name must be 1 to 80 characters"));
    }
    let key = new_api_key();
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO api_keys (workspace_id, created_by, name, prefix, hash)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(principal.workspace_id())
    .bind(principal.user_id())
    .bind(&name)
    .bind(&key.prefix)
    .bind(&key.hash)
    .fetch_one(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(IssuedApiKey {
            id,
            key: key.secret,
            key_prefix: key.prefix,
        }),
    ))
}

/// Revoke a key. Requests with it fail from now on.
#[utoipa::path(
    delete, path = "/v1/api-keys/{keyId}", tag = "API keys", operation_id = "revoke_api_key",
    summary = "Revoke an API key", security(("bearerAuth" = [])),
    params(("keyId" = Uuid, Path, description = "The key to revoke")),
    responses(
        (status = 204, description = "Revoked"),
        (status = 401, body = ApiErrorBody),
        (status = 403, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn revoke_api_key(
    State(state): State<AppState>,
    principal: Principal,
    Path(key_id): Path<Uuid>,
) -> Result<StatusCode> {
    let owner = match &principal {
        Principal::Key(_) => return Err(ApiError(Error::ForbiddenForApiKey)),
        Principal::Session(s) if s.role == Role::Owner => true,
        Principal::Session(_) => false,
    };
    if !owner {
        return Err(ApiError(Error::Forbidden));
    }
    let affected = sqlx::query(
        "UPDATE api_keys SET revoked_at = now() WHERE id = $1 AND workspace_id = $2 AND revoked_at IS NULL",
    )
    .bind(key_id)
    .bind(principal.workspace_id())
    .execute(&state.pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(ApiError(Error::NotFound("api key")));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[allow(dead_code)]
fn _schema_used(_: IssueApiKey) {}
