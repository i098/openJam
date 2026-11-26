//! `/v1/notification-rules` and the inbox: which events reach whom, and the bell list.

use crate::error::{ApiError, ApiErrorBody, Error, Result};
use crate::{auth::Principal, state::AppState};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, NaiveTime, Utc};
use openjam_core::events::CATEGORIES;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use std::str::FromStr;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Channels that deliver today. `chat` is the in-app inbox; `webhook` posts to your webhooks.
pub const CHANNELS: [&str; 2] = ["chat", "webhook"];

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QuietHours {
    /// Local time, `HH:MM`.
    pub start: String,
    /// Local time, `HH:MM`. Earlier than `start` means the window crosses midnight.
    pub end: String,
    /// An IANA zone, for example `Europe/Berlin`.
    pub timezone: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotificationRule {
    /// `action_required`, `error`, `status_update`, `insight` or `completion`.
    pub category: String,
    pub channels: Vec<String>,
    pub is_active: bool,
    /// Merge inbox notifications of this category that arrive within the window.
    pub batching_enabled: bool,
    pub batch_window_minutes: i32,
    /// Webhook deliveries wait until the window ends.
    pub quiet_hours: Option<QuietHours>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NotificationRuleList {
    pub rules: Vec<NotificationRule>,
}

#[derive(sqlx::FromRow)]
struct RuleRow {
    category: String,
    channels: Vec<String>,
    is_active: bool,
    batching_enabled: bool,
    batch_window_minutes: i32,
    quiet_start: Option<String>,
    quiet_end: Option<String>,
    timezone: Option<String>,
}

impl NotificationRule {
    /// The rule a workspace has until it changes one: everything on, to both channels.
    pub fn default_for(category: &str) -> Self {
        Self {
            category: category.to_string(),
            channels: CHANNELS.iter().map(|c| c.to_string()).collect(),
            is_active: true,
            batching_enabled: false,
            batch_window_minutes: 60,
            quiet_hours: None,
        }
    }
}

impl From<RuleRow> for NotificationRule {
    fn from(r: RuleRow) -> Self {
        let quiet_hours = match (r.quiet_start, r.quiet_end, r.timezone) {
            (Some(start), Some(end), Some(timezone)) => Some(QuietHours {
                start,
                end,
                timezone,
            }),
            _ => None,
        };
        Self {
            category: r.category,
            channels: r.channels,
            is_active: r.is_active,
            batching_enabled: r.batching_enabled,
            batch_window_minutes: r.batch_window_minutes,
            quiet_hours,
        }
    }
}

const RULE_COLUMNS: &str = "category, channels, is_active, batching_enabled, batch_window_minutes, quiet_start, quiet_end, timezone";

/// The rule for one category: the stored one, or the default.
pub async fn rule_for(
    pool: &PgPool,
    workspace_id: Uuid,
    category: &str,
) -> sqlx::Result<NotificationRule> {
    let row: Option<RuleRow> = sqlx::query_as(&format!(
        "SELECT {RULE_COLUMNS} FROM notification_rules WHERE workspace_id = $1 AND category = $2"
    ))
    .bind(workspace_id)
    .bind(category)
    .fetch_optional(pool)
    .await?;
    Ok(row.map_or_else(|| NotificationRule::default_for(category), Into::into))
}

/// All five categories, stored or default.
#[utoipa::path(
    get, path = "/v1/notification-rules", tag = "Notifications", operation_id = "list_notification_rules",
    summary = "Notification rules", security(("bearerAuth" = [])),
    responses((status = 200, body = NotificationRuleList), (status = 401, body = ApiErrorBody))
)]
pub async fn list_notification_rules(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<NotificationRuleList>> {
    let mut rules = Vec::new();
    for category in CATEGORIES {
        rules.push(rule_for(&state.pool, principal.workspace_id(), category).await?);
    }
    Ok(Json(NotificationRuleList { rules }))
}

/// `Some(None)` is `null` in the body (clear), `None` is a missing field (keep).
fn set_or_clear<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Option<QuietHours>>, D::Error> {
    Option::<QuietHours>::deserialize(d).map(Some)
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateNotificationRule {
    pub channels: Option<Vec<String>>,
    pub is_active: Option<bool>,
    pub batching_enabled: Option<bool>,
    pub batch_window_minutes: Option<i32>,
    /// An object sets quiet hours. `null` clears them. Leave it out to keep them.
    #[serde(default, deserialize_with = "set_or_clear")]
    #[schema(value_type = Option<QuietHours>)]
    pub quiet_hours: Option<Option<QuietHours>>,
}

pub fn parse_clock(text: &str) -> Option<NaiveTime> {
    if text.len() != 5 {
        return None;
    }
    NaiveTime::parse_from_str(text, "%H:%M").ok()
}

fn validate(update: &UpdateNotificationRule) -> Result<()> {
    if let Some(channels) = &update.channels
        && let Some(bad) = channels.iter().find(|c| !CHANNELS.contains(&c.as_str()))
    {
        return Err(ApiError::bad(format!(
            "channel `{bad}` is not available; use {}",
            CHANNELS.join(" or ")
        )));
    }
    if update
        .batch_window_minutes
        .is_some_and(|m| !(1..=1440).contains(&m))
    {
        return Err(ApiError::bad("batchWindowMinutes must be 1 to 1440"));
    }
    if let Some(Some(q)) = &update.quiet_hours {
        if parse_clock(&q.start).is_none() || parse_clock(&q.end).is_none() {
            return Err(ApiError::bad("quiet hours start and end must be HH:MM"));
        }
        if chrono_tz::Tz::from_str(&q.timezone).is_err() {
            return Err(ApiError::bad("quiet hours timezone must be an IANA zone"));
        }
    }
    Ok(())
}

/// Change one category. Fields you leave out keep their value.
#[utoipa::path(
    patch, path = "/v1/notification-rules/{category}", tag = "Notifications", operation_id = "update_notification_rule",
    summary = "Change a notification rule", security(("bearerAuth" = [])),
    params(("category" = String, Path, description = "`action_required`, `error`, `status_update`, `insight` or `completion`")),
    request_body = UpdateNotificationRule,
    responses(
        (status = 200, body = NotificationRule),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn update_notification_rule(
    State(state): State<AppState>,
    principal: Principal,
    Path(category): Path<String>,
    Json(update): Json<UpdateNotificationRule>,
) -> Result<Json<NotificationRule>> {
    if !CATEGORIES.contains(&category.as_str()) {
        return Err(ApiError(Error::NotFound("notification category")));
    }
    validate(&update)?;
    let workspace_id = principal.workspace_id();
    let current = rule_for(&state.pool, workspace_id, &category).await?;
    let quiet = match update.quiet_hours {
        Some(q) => q,
        None => current.quiet_hours,
    };
    let row: RuleRow = sqlx::query_as(&format!(
        "INSERT INTO notification_rules
             (workspace_id, category, channels, is_active, batching_enabled, batch_window_minutes,
              quiet_start, quiet_end, timezone)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (workspace_id, category) DO UPDATE SET
             channels = $3, is_active = $4, batching_enabled = $5, batch_window_minutes = $6,
             quiet_start = $7, quiet_end = $8, timezone = $9
         RETURNING {RULE_COLUMNS}"
    ))
    .bind(workspace_id)
    .bind(&category)
    .bind(update.channels.unwrap_or(current.channels))
    .bind(update.is_active.unwrap_or(current.is_active))
    .bind(update.batching_enabled.unwrap_or(current.batching_enabled))
    .bind(
        update
            .batch_window_minutes
            .unwrap_or(current.batch_window_minutes),
    )
    .bind(quiet.as_ref().map(|q| q.start.clone()))
    .bind(quiet.as_ref().map(|q| q.end.clone()))
    .bind(quiet.as_ref().map(|q| q.timezone.clone()))
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(row.into()))
}

// ---- inbox ----------------------------------------------------------------------------------

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub id: Uuid,
    /// The event type, for example `scan.complete`. `batch.<category>` for a merged one.
    pub kind: String,
    pub category: Option<String>,
    pub body: Value,
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotificationList {
    pub notifications: Vec<Notification>,
    /// Unread in this workspace, not only on this page.
    pub unread_count: i64,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct NotificationsQuery {
    /// `true`: only unread ones.
    pub unread: Option<bool>,
    /// 1 to 100. The default is 30.
    pub limit: Option<i64>,
}

/// The signed-in person's notifications in this workspace, newest first.
#[utoipa::path(
    get, path = "/v1/notifications", tag = "Notifications", operation_id = "list_notifications",
    summary = "The notification inbox", security(("bearerAuth" = [])), params(NotificationsQuery),
    responses((status = 200, body = NotificationList), (status = 401, body = ApiErrorBody))
)]
pub async fn list_notifications(
    State(state): State<AppState>,
    principal: Principal,
    Query(q): Query<NotificationsQuery>,
) -> Result<Json<NotificationList>> {
    let (user_id, workspace_id) = (principal.user_id(), principal.workspace_id());
    let notifications = sqlx::query_as(
        "SELECT id, kind, category, body, read_at, created_at FROM notifications
         WHERE user_id = $1 AND workspace_id = $2 AND (NOT $3 OR read_at IS NULL)
         ORDER BY created_at DESC, id LIMIT $4",
    )
    .bind(user_id)
    .bind(workspace_id)
    .bind(q.unread.unwrap_or(false))
    .bind(q.limit.unwrap_or(30).clamp(1, 100))
    .fetch_all(&state.pool)
    .await?;
    let unread_count = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id = $1 AND workspace_id = $2 AND read_at IS NULL",
    )
    .bind(user_id)
    .bind(workspace_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(NotificationList {
        notifications,
        unread_count,
    }))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct MarkRead {
    /// These notifications. Leave it out to mark every unread one.
    pub ids: Option<Vec<Uuid>>,
}

/// Mark notifications read.
#[utoipa::path(
    post, path = "/v1/notifications/read", tag = "Notifications", operation_id = "mark_notifications_read",
    summary = "Mark notifications read", security(("bearerAuth" = [])),
    request_body = Option<MarkRead>,
    responses((status = 204, description = "Marked"), (status = 401, body = ApiErrorBody))
)]
pub async fn mark_notifications_read(
    State(state): State<AppState>,
    principal: Principal,
    body: Option<Json<MarkRead>>,
) -> Result<StatusCode> {
    let ids = body.and_then(|Json(b)| b.ids);
    sqlx::query(
        "UPDATE notifications SET read_at = now()
         WHERE user_id = $1 AND workspace_id = $2 AND read_at IS NULL AND ($3::uuid[] IS NULL OR id = ANY($3))",
    )
    .bind(principal.user_id())
    .bind(principal.workspace_id())
    .bind(ids)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
