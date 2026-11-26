//! Contract stubs. Every `/v1` operation in the source API exists here and answers 501
//! `not_implemented`, so later slices replace the handler without a route change.

use crate::error::ApiErrorBody;

// Every stub takes `Principal`, so a caller without credentials gets 401, not 501.
#[utoipa::path(
    get, path = "/v1/brand", tag = "Brand", operation_id = "get_brand",
    summary = "Get /v1/brand (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_brand(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    patch, path = "/v1/brand", tag = "Brand", operation_id = "update_brand",
    summary = "Patch /v1/brand (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn update_brand(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    put, path = "/v1/brand/competitors", tag = "Brand", operation_id = "pin_competitors",
    summary = "Put /v1/brand/competitors (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn pin_competitors(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    delete, path = "/v1/brand/competitors", tag = "Brand", operation_id = "unpin_competitors",
    summary = "Delete /v1/brand/competitors (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn unpin_competitors(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/brand/competitors", tag = "Brand", operation_id = "list_brand_competitors",
    summary = "Get /v1/brand/competitors (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_brand_competitors(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/brand/competitors", tag = "Brand", operation_id = "add_brand_competitors",
    summary = "Post /v1/brand/competitors (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn add_brand_competitors(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/brand/recalibrations", tag = "Brand", operation_id = "recalibrate_brand",
    summary = "Post /v1/brand/recalibrations (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn recalibrate_brand(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/prompts", tag = "Prompts", operation_id = "list_prompts",
    summary = "Get /v1/prompts (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_prompts(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/prompts", tag = "Prompts", operation_id = "add_prompt",
    summary = "Post /v1/prompts (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn add_prompt(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    put, path = "/v1/prompts", tag = "Prompts", operation_id = "edit_prompts_bulk",
    summary = "Put /v1/prompts (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn edit_prompts_bulk(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/prompts/cap", tag = "Prompts", operation_id = "get_prompt_cap",
    summary = "Get /v1/prompts/cap (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_prompt_cap(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/prompts/topics", tag = "Prompts", operation_id = "list_prompt_topics",
    summary = "Get /v1/prompts/topics (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_prompt_topics(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    delete, path = "/v1/prompts/topics/{topic}", tag = "Prompts", operation_id = "remove_prompt_topic",
    summary = "Delete /v1/prompts/topics/{topic} (not built yet)",
        security(("bearerAuth" = [])),
    params(("topic" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn remove_prompt_topic(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/prompts/suggestions", tag = "Prompts", operation_id = "suggest_prompts",
    summary = "Post /v1/prompts/suggestions (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn suggest_prompts(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/prompts/suggestions", tag = "Prompts", operation_id = "get_prompt_suggestion",
    summary = "Get /v1/prompts/suggestions (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_prompt_suggestion(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    delete, path = "/v1/prompts/suggestions", tag = "Prompts", operation_id = "cancel_prompt_suggestion",
    summary = "Delete /v1/prompts/suggestions (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn cancel_prompt_suggestion(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    patch, path = "/v1/prompts/{promptId}", tag = "Prompts", operation_id = "edit_prompt",
    summary = "Patch /v1/prompts/{promptId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("promptId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn edit_prompt(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    delete, path = "/v1/prompts/{promptId}", tag = "Prompts", operation_id = "remove_prompt",
    summary = "Delete /v1/prompts/{promptId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("promptId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn remove_prompt(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/prompts/{promptId}", tag = "Prompts", operation_id = "get_prompt",
    summary = "Get /v1/prompts/{promptId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("promptId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_prompt(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/prompts/bulk", tag = "Prompts", operation_id = "add_prompts_bulk",
    summary = "Post /v1/prompts/bulk (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn add_prompts_bulk(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/prompts/{promptId}/pause", tag = "Prompts", operation_id = "pause_prompt",
    summary = "Post /v1/prompts/{promptId}/pause (not built yet)",
        security(("bearerAuth" = [])),
    params(("promptId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn pause_prompt(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/prompts/{promptId}/replacements", tag = "Prompts", operation_id = "replace_prompt",
    summary = "Post /v1/prompts/{promptId}/replacements (not built yet)",
        security(("bearerAuth" = [])),
    params(("promptId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn replace_prompt(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/plan-batches", tag = "Planning", operation_id = "create_plan_batch",
    summary = "Post /v1/plan-batches (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn create_plan_batch(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/visibility-runs", tag = "Visibility runs", operation_id = "create_visibility_run",
    summary = "Post /v1/visibility-runs (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn create_visibility_run(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/visibility-runs", tag = "Visibility runs", operation_id = "list_visibility_runs",
    summary = "Get /v1/visibility-runs (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_visibility_runs(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/visibility-runs/{runId}", tag = "Visibility runs", operation_id = "get_visibility_run",
    summary = "Get /v1/visibility-runs/{runId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("runId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_visibility_run(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/credits", tag = "Credits", operation_id = "get_credits",
    summary = "Get /v1/credits (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_credits(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/pricing", tag = "Pricing", operation_id = "get_pricing",
    summary = "Get /v1/pricing (not built yet)",

    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_pricing() -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/site-changes/{changeId}", tag = "Site changes", operation_id = "get_site_change",
    summary = "Get /v1/site-changes/{changeId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("changeId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_site_change(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    delete, path = "/v1/site-changes/{changeId}", tag = "Site changes", operation_id = "discard_site_change",
    summary = "Delete /v1/site-changes/{changeId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("changeId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn discard_site_change(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/site-changes/{changeId}/pull-requests", tag = "Site changes", operation_id = "create_site_change_pull_request",
    summary = "Post /v1/site-changes/{changeId}/pull-requests (not built yet)",
        security(("bearerAuth" = [])),
    params(("changeId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn create_site_change_pull_request(
    _principal: crate::auth::Principal,
) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/projects/{projectId}", tag = "Projects", operation_id = "get_project",
    summary = "Get /v1/projects/{projectId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("projectId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_project(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    patch, path = "/v1/brand/competitors/{name}", tag = "Brand", operation_id = "update_brand_competitor",
    summary = "Patch /v1/brand/competitors/{name} (not built yet)",
        security(("bearerAuth" = [])),
    params(("name" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn update_brand_competitor(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    delete, path = "/v1/brand/competitors/{name}", tag = "Brand", operation_id = "remove_brand_competitor",
    summary = "Delete /v1/brand/competitors/{name} (not built yet)",
        security(("bearerAuth" = [])),
    params(("name" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn remove_brand_competitor(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/billing", tag = "Billing", operation_id = "get_billing",
    summary = "Get /v1/billing (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_billing(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    put, path = "/v1/usage/cap", tag = "Usage", operation_id = "set_usage_cap",
    summary = "Put /v1/usage/cap (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn set_usage_cap(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/engines", tag = "Engines", operation_id = "get_engines",
    summary = "Get /v1/engines (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_engines(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    put, path = "/v1/engines", tag = "Engines", operation_id = "put_engines",
    summary = "Put /v1/engines (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn put_engines(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

pub fn stub_routes() -> utoipa_axum::router::OpenApiRouter<crate::state::AppState> {
    utoipa_axum::router::OpenApiRouter::new()
        .routes(utoipa_axum::routes!(get_brand))
        .routes(utoipa_axum::routes!(update_brand))
        .routes(utoipa_axum::routes!(pin_competitors))
        .routes(utoipa_axum::routes!(unpin_competitors))
        .routes(utoipa_axum::routes!(list_brand_competitors))
        .routes(utoipa_axum::routes!(add_brand_competitors))
        .routes(utoipa_axum::routes!(recalibrate_brand))
        .routes(utoipa_axum::routes!(list_prompts))
        .routes(utoipa_axum::routes!(add_prompt))
        .routes(utoipa_axum::routes!(edit_prompts_bulk))
        .routes(utoipa_axum::routes!(get_prompt_cap))
        .routes(utoipa_axum::routes!(list_prompt_topics))
        .routes(utoipa_axum::routes!(remove_prompt_topic))
        .routes(utoipa_axum::routes!(suggest_prompts))
        .routes(utoipa_axum::routes!(get_prompt_suggestion))
        .routes(utoipa_axum::routes!(cancel_prompt_suggestion))
        .routes(utoipa_axum::routes!(edit_prompt))
        .routes(utoipa_axum::routes!(remove_prompt))
        .routes(utoipa_axum::routes!(get_prompt))
        .routes(utoipa_axum::routes!(add_prompts_bulk))
        .routes(utoipa_axum::routes!(pause_prompt))
        .routes(utoipa_axum::routes!(replace_prompt))
        .routes(utoipa_axum::routes!(create_plan_batch))
        .routes(utoipa_axum::routes!(create_visibility_run))
        .routes(utoipa_axum::routes!(list_visibility_runs))
        .routes(utoipa_axum::routes!(get_visibility_run))
        .routes(utoipa_axum::routes!(get_credits))
        .routes(utoipa_axum::routes!(get_pricing))
        .routes(utoipa_axum::routes!(get_site_change))
        .routes(utoipa_axum::routes!(discard_site_change))
        .routes(utoipa_axum::routes!(create_site_change_pull_request))
        .routes(utoipa_axum::routes!(get_project))
        .routes(utoipa_axum::routes!(update_brand_competitor))
        .routes(utoipa_axum::routes!(remove_brand_competitor))
        .routes(utoipa_axum::routes!(get_billing))
        .routes(utoipa_axum::routes!(set_usage_cap))
        .routes(utoipa_axum::routes!(get_engines))
        .routes(utoipa_axum::routes!(put_engines))
}
