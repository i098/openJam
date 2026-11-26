//! `/v1/webhooks`: where openJam sends signed event deliveries.

use crate::error::{ApiError, ApiErrorBody, Result};
use crate::{auth::Principal, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use openjam_core::auth::new_prefixed_token;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Webhook {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookList {
    pub webhooks: Vec<Webhook>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWebhook {
    pub name: String,
    /// An `http` or `https` address that accepts `POST`.
    pub url: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreatedWebhook {
    #[serde(flatten)]
    pub webhook: Webhook,
    /// Verifies the `X-OpenJam-Signature` header. Shown once; store it now.
    pub signing_secret: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateWebhook {
    pub name: Option<String>,
    pub url: Option<String>,
    pub enabled: Option<bool>,
}

fn valid_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError::bad("name must be 1 to 80 characters"));
    }
    Ok(name.to_string())
}

fn valid_url(raw: &str) -> Result<String> {
    let raw = raw.trim();
    let ok = raw.len() <= 2000
        && url::Url::parse(raw).is_ok_and(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.host_str().is_some()
                && u.username().is_empty()
        });
    if !ok {
        return Err(ApiError::bad(
            "url must be an http or https address without a user name",
        ));
    }
    Ok(raw.to_string())
}

const COLUMNS: &str = "id, name, url, enabled, created_at";

/// Every webhook of the workspace.
#[utoipa::path(
    get, path = "/v1/webhooks", tag = "Webhooks", operation_id = "list_webhooks",
    summary = "List webhooks", security(("bearerAuth" = [])),
    responses((status = 200, body = WebhookList), (status = 401, body = ApiErrorBody))
)]
pub async fn list_webhooks(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<WebhookList>> {
    let webhooks = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM webhooks WHERE workspace_id = $1 ORDER BY created_at"
    ))
    .bind(principal.workspace_id())
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(WebhookList { webhooks }))
}

/// Add a webhook. Each event of the workspace is posted to it, signed with the returned secret.
#[utoipa::path(
    post, path = "/v1/webhooks", tag = "Webhooks", operation_id = "create_webhook",
    summary = "Add a webhook", security(("bearerAuth" = [])), request_body = CreateWebhook,
    responses(
        (status = 201, body = CreatedWebhook),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody)
    )
)]
pub async fn create_webhook(
    State(state): State<AppState>,
    principal: Principal,
    Json(body): Json<CreateWebhook>,
) -> Result<(StatusCode, Json<CreatedWebhook>)> {
    let name = valid_name(&body.name)?;
    let url = valid_url(&body.url)?;
    let secret = new_prefixed_token("whsec_", 32);
    let webhook: Webhook = sqlx::query_as(&format!(
        "INSERT INTO webhooks (workspace_id, name, url, secret) VALUES ($1, $2, $3, $4)
         RETURNING {COLUMNS}"
    ))
    .bind(principal.workspace_id())
    .bind(name)
    .bind(url)
    .bind(&secret)
    .fetch_one(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedWebhook {
            webhook,
            signing_secret: secret,
        }),
    ))
}

/// Rename a webhook, point it elsewhere, or turn it on or off.
#[utoipa::path(
    patch, path = "/v1/webhooks/{webhookId}", tag = "Webhooks", operation_id = "update_webhook",
    summary = "Rename, enable or disable a webhook", security(("bearerAuth" = [])),
    params(("webhookId" = Uuid, Path, description = "The webhook to change")),
    request_body = UpdateWebhook,
    responses(
        (status = 200, body = Webhook),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn update_webhook(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateWebhook>,
) -> Result<Json<Webhook>> {
    let name = body.name.as_deref().map(valid_name).transpose()?;
    let url = body.url.as_deref().map(valid_url).transpose()?;
    let webhook: Option<Webhook> = sqlx::query_as(&format!(
        "UPDATE webhooks SET name = COALESCE($3, name), url = COALESCE($4, url),
             enabled = COALESCE($5, enabled)
         WHERE id = $1 AND workspace_id = $2 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(principal.workspace_id())
    .bind(name)
    .bind(url)
    .bind(body.enabled)
    .fetch_optional(&state.pool)
    .await?;
    webhook
        .map(Json)
        .ok_or(ApiError(crate::error::Error::NotFound("webhook")))
}

/// Remove a webhook and its delivery history.
#[utoipa::path(
    delete, path = "/v1/webhooks/{webhookId}", tag = "Webhooks", operation_id = "delete_webhook",
    summary = "Remove a webhook", security(("bearerAuth" = [])),
    params(("webhookId" = Uuid, Path, description = "The webhook to remove")),
    responses(
        (status = 204, description = "Removed"),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn delete_webhook(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    let removed = sqlx::query("DELETE FROM webhooks WHERE id = $1 AND workspace_id = $2")
        .bind(id)
        .bind(principal.workspace_id())
        .execute(&state.pool)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(ApiError(crate::error::Error::NotFound("webhook")));
    }
    Ok(StatusCode::NO_CONTENT)
}
