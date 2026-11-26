use crate::error::Result;
use crate::{auth::Principal, error::ApiErrorBody, state::AppState, types::*};
use axum::{Json, extract::State};

/// The workspace and user the credentials belong to.
#[utoipa::path(
    get, path = "/v1/me", tag = "Meta", operation_id = "get_me",
    summary = "The authenticated workspace and key owner", security(("bearerAuth" = [])),
    responses((status = 200, body = Me), (status = 401, body = ApiErrorBody))
)]
pub async fn get_me(State(state): State<AppState>, principal: Principal) -> Result<Json<Me>> {
    Ok(Json(principal.me(&state.pool).await?))
}

/// The usage ledger: scans today and cost, with the optional daily scan cap.
#[utoipa::path(
    get, path = "/v1/usage", tag = "Usage", operation_id = "get_usage",
    summary = "Usage and the daily scan cap", security(("bearerAuth" = [])),
    responses((status = 200, body = Usage), (status = 401, body = ApiErrorBody))
)]
pub async fn get_usage(State(state): State<AppState>, principal: Principal) -> Result<Json<Usage>> {
    let workspace_id = principal.workspace_id();
    let (scans_today, today, month): (i64, f64, f64) = sqlx::query_as(
        "SELECT
             count(*) FILTER (WHERE kind = 'scan'),
             COALESCE(sum(cost_cents) FILTER (WHERE at >= date_trunc('day', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'), 0)::float8,
             COALESCE(sum(cost_cents) FILTER (WHERE at >= date_trunc('month', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'), 0)::float8
         FROM usage_ledger WHERE workspace_id = $1",
    )
    .bind(workspace_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(Usage {
        scans_today,
        scan_cap_per_day: state.config.scan_cap_per_day,
        cost_cents_today: today,
        cost_cents_month: month,
    }))
}
