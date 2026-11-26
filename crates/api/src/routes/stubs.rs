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
    get, path = "/v1/opportunities", tag = "Opportunities", operation_id = "list_opportunities",
    summary = "Get /v1/opportunities (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_opportunities(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/opportunities/{opportunityId}", tag = "Opportunities", operation_id = "get_opportunity",
    summary = "Get /v1/opportunities/{opportunityId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("opportunityId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_opportunity(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/opportunities/{opportunityId}/dismissal", tag = "Opportunities", operation_id = "dismiss_opportunity",
    summary = "Post /v1/opportunities/{opportunityId}/dismissal (not built yet)",
        security(("bearerAuth" = [])),
    params(("opportunityId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn dismiss_opportunity(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    delete, path = "/v1/opportunities/{opportunityId}/dismissal", tag = "Opportunities", operation_id = "restore_opportunity",
    summary = "Delete /v1/opportunities/{opportunityId}/dismissal (not built yet)",
        security(("bearerAuth" = [])),
    params(("opportunityId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn restore_opportunity(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/opportunities/{opportunityId}/plan-requests", tag = "Opportunities", operation_id = "request_opportunity_plan",
    summary = "Post /v1/opportunities/{opportunityId}/plan-requests (not built yet)",
        security(("bearerAuth" = [])),
    params(("opportunityId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn request_opportunity_plan(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/opportunities/{opportunityId}/brief", tag = "Opportunities", operation_id = "get_opportunity_brief",
    summary = "Get /v1/opportunities/{opportunityId}/brief (not built yet)",
        security(("bearerAuth" = [])),
    params(("opportunityId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_opportunity_brief(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/opportunities/{opportunityId}/claims", tag = "Opportunities", operation_id = "claim_opportunity",
    summary = "Post /v1/opportunities/{opportunityId}/claims (not built yet)",
        security(("bearerAuth" = [])),
    params(("opportunityId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn claim_opportunity(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    delete, path = "/v1/opportunities/{opportunityId}/claims/current", tag = "Opportunities", operation_id = "release_opportunity_claim",
    summary = "Delete /v1/opportunities/{opportunityId}/claims/current (not built yet)",
        security(("bearerAuth" = [])),
    params(("opportunityId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn release_opportunity_claim(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/opportunities/{opportunityId}/agent-failures", tag = "Opportunities", operation_id = "report_opportunity_agent_failure",
    summary = "Post /v1/opportunities/{opportunityId}/agent-failures (not built yet)",
        security(("bearerAuth" = [])),
    params(("opportunityId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn report_opportunity_agent_failure(
    _principal: crate::auth::Principal,
) -> crate::Result<()> {
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
    post, path = "/v1/content", tag = "Content", operation_id = "create_content",
    summary = "Post /v1/content (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn create_content(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/content", tag = "Content", operation_id = "list_content",
    summary = "Get /v1/content (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_content(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/content/{contentId}", tag = "Content", operation_id = "get_content",
    summary = "Get /v1/content/{contentId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_content(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    delete, path = "/v1/content/{contentId}", tag = "Content", operation_id = "discard_content",
    summary = "Delete /v1/content/{contentId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn discard_content(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/content/{contentId}/restorations", tag = "Content", operation_id = "restore_content",
    summary = "Post /v1/content/{contentId}/restorations (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn restore_content(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/content/{contentId}/regenerations", tag = "Content", operation_id = "regenerate_content",
    summary = "Post /v1/content/{contentId}/regenerations (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn regenerate_content(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/content/{contentId}/edit-requests", tag = "Content", operation_id = "request_content_edit",
    summary = "Post /v1/content/{contentId}/edit-requests (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn request_content_edit(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/content/{contentId}/revisions", tag = "Revisions", operation_id = "list_content_revisions",
    summary = "Get /v1/content/{contentId}/revisions (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_content_revisions(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/content/{contentId}/revisions", tag = "Revisions", operation_id = "add_content_revision",
    summary = "Post /v1/content/{contentId}/revisions (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn add_content_revision(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/content/{contentId}/revisions/{revisionId}", tag = "Revisions", operation_id = "get_content_revision",
    summary = "Get /v1/content/{contentId}/revisions/{revisionId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter"), ("revisionId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_content_revision(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/content/{contentId}/destinations", tag = "Publishing", operation_id = "list_content_destinations",
    summary = "Get /v1/content/{contentId}/destinations (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_content_destinations(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    put, path = "/v1/content/{contentId}/destinations/{destinationId}", tag = "Publishing", operation_id = "publish_content",
    summary = "Put /v1/content/{contentId}/destinations/{destinationId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter"), ("destinationId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn publish_content(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    delete, path = "/v1/content/{contentId}/destinations/{destinationId}", tag = "Publishing", operation_id = "cancel_content_publishing",
    summary = "Delete /v1/content/{contentId}/destinations/{destinationId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter"), ("destinationId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn cancel_content_publishing(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/content/{contentId}/deliveries", tag = "Deliveries", operation_id = "list_content_deliveries",
    summary = "Get /v1/content/{contentId}/deliveries (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_content_deliveries(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/content/{contentId}/publications", tag = "Publications", operation_id = "list_content_publications",
    summary = "Get /v1/content/{contentId}/publications (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_content_publications(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/content/{contentId}/publications", tag = "Publications", operation_id = "mark_content_published",
    summary = "Post /v1/content/{contentId}/publications (not built yet)",
        security(("bearerAuth" = [])),
    params(("contentId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn mark_content_published(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/destinations", tag = "Destinations", operation_id = "list_destinations",
    summary = "Get /v1/destinations (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_destinations(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/destinations", tag = "Destinations", operation_id = "create_destination",
    summary = "Post /v1/destinations (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn create_destination(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    patch, path = "/v1/destinations/{destinationId}", tag = "Destinations", operation_id = "update_destination",
    summary = "Patch /v1/destinations/{destinationId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("destinationId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn update_destination(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    delete, path = "/v1/destinations/{destinationId}", tag = "Destinations", operation_id = "remove_destination",
    summary = "Delete /v1/destinations/{destinationId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("destinationId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn remove_destination(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    put, path = "/v1/destinations/{destinationId}/github", tag = "Destinations", operation_id = "configure_destination_github",
    summary = "Put /v1/destinations/{destinationId}/github (not built yet)",
        security(("bearerAuth" = [])),
    params(("destinationId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn configure_destination_github(_principal: crate::auth::Principal) -> crate::Result<()> {
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
    get, path = "/v1/actions", tag = "Actions", operation_id = "list_actions",
    summary = "Get /v1/actions (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_actions(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/actions/{actionId}", tag = "Actions", operation_id = "get_action",
    summary = "Get /v1/actions/{actionId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("actionId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_action(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/actions/{actionId}/completion", tag = "Actions", operation_id = "complete_action",
    summary = "Post /v1/actions/{actionId}/completion (not built yet)",
        security(("bearerAuth" = [])),
    params(("actionId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn complete_action(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/actions/{actionId}/start", tag = "Actions", operation_id = "start_action",
    summary = "Post /v1/actions/{actionId}/start (not built yet)",
        security(("bearerAuth" = [])),
    params(("actionId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn start_action(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/actions/{actionId}/sent", tag = "Actions", operation_id = "report_action_sent",
    summary = "Post /v1/actions/{actionId}/sent (not built yet)",
        security(("bearerAuth" = [])),
    params(("actionId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn report_action_sent(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/actions/{actionId}/dismissal", tag = "Actions", operation_id = "dismiss_action",
    summary = "Post /v1/actions/{actionId}/dismissal (not built yet)",
        security(("bearerAuth" = [])),
    params(("actionId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn dismiss_action(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    delete, path = "/v1/actions/{actionId}/dismissal", tag = "Actions", operation_id = "restore_action",
    summary = "Delete /v1/actions/{actionId}/dismissal (not built yet)",
        security(("bearerAuth" = [])),
    params(("actionId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn restore_action(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/actions/{actionId}/checks", tag = "Actions", operation_id = "check_action_again",
    summary = "Post /v1/actions/{actionId}/checks (not built yet)",
        security(("bearerAuth" = [])),
    params(("actionId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn check_action_again(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/actions/{actionId}/executions", tag = "Actions", operation_id = "create_action_execution",
    summary = "Post /v1/actions/{actionId}/executions (not built yet)",
        security(("bearerAuth" = [])),
    params(("actionId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn create_action_execution(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/action-completions/{token}", tag = "Actions", operation_id = "complete_action_with_token",
    summary = "Post /v1/action-completions/{token} (not built yet)",
        security(("bearerAuth" = [])),
    params(("token" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn complete_action_with_token(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/agent-connect", tag = "Agent connect", operation_id = "start_agent_connect",
    summary = "Post /v1/agent-connect (not built yet)",

    responses((status = 501, body = ApiErrorBody))
)]
pub async fn start_agent_connect() -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/agent-connect/{pollToken}", tag = "Agent connect", operation_id = "get_agent_connect",
    summary = "Get /v1/agent-connect/{pollToken} (not built yet)",

    params(("pollToken" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_agent_connect() -> crate::Result<()> {
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
    get, path = "/v1/research", tag = "Research", operation_id = "get_research",
    summary = "Get /v1/research (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_research(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/research/report", tag = "Research", operation_id = "get_research_report",
    summary = "Get /v1/research/report (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn get_research_report(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/research-requests", tag = "Research", operation_id = "create_research_request",
    summary = "Post /v1/research-requests (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn create_research_request(_principal: crate::auth::Principal) -> crate::Result<()> {
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
    get, path = "/v1/webhooks", tag = "Webhooks", operation_id = "list_webhooks",
    summary = "Get /v1/webhooks (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_webhooks(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    post, path = "/v1/webhooks", tag = "Webhooks", operation_id = "create_webhook",
    summary = "Post /v1/webhooks (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn create_webhook(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    patch, path = "/v1/webhooks/{webhookId}", tag = "Webhooks", operation_id = "update_webhook",
    summary = "Patch /v1/webhooks/{webhookId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("webhookId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn update_webhook(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    delete, path = "/v1/webhooks/{webhookId}", tag = "Webhooks", operation_id = "delete_webhook",
    summary = "Delete /v1/webhooks/{webhookId} (not built yet)",
        security(("bearerAuth" = [])),
    params(("webhookId" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn delete_webhook(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    get, path = "/v1/notification-rules", tag = "Notifications", operation_id = "list_notification_rules",
    summary = "Get /v1/notification-rules (not built yet)",
        security(("bearerAuth" = [])),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn list_notification_rules(_principal: crate::auth::Principal) -> crate::Result<()> {
    Err(crate::ApiError(openjam_core::Error::NotImplemented))
}

#[utoipa::path(
    patch, path = "/v1/notification-rules/{category}", tag = "Notifications", operation_id = "update_notification_rule",
    summary = "Patch /v1/notification-rules/{category} (not built yet)",
        security(("bearerAuth" = [])),
    params(("category" = String, Path, description = "Path parameter")),
    responses((status = 501, body = ApiErrorBody))
)]
pub async fn update_notification_rule(_principal: crate::auth::Principal) -> crate::Result<()> {
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
        .routes(utoipa_axum::routes!(list_opportunities))
        .routes(utoipa_axum::routes!(get_opportunity))
        .routes(utoipa_axum::routes!(dismiss_opportunity))
        .routes(utoipa_axum::routes!(restore_opportunity))
        .routes(utoipa_axum::routes!(request_opportunity_plan))
        .routes(utoipa_axum::routes!(get_opportunity_brief))
        .routes(utoipa_axum::routes!(claim_opportunity))
        .routes(utoipa_axum::routes!(release_opportunity_claim))
        .routes(utoipa_axum::routes!(report_opportunity_agent_failure))
        .routes(utoipa_axum::routes!(create_plan_batch))
        .routes(utoipa_axum::routes!(create_content))
        .routes(utoipa_axum::routes!(list_content))
        .routes(utoipa_axum::routes!(get_content))
        .routes(utoipa_axum::routes!(discard_content))
        .routes(utoipa_axum::routes!(restore_content))
        .routes(utoipa_axum::routes!(regenerate_content))
        .routes(utoipa_axum::routes!(request_content_edit))
        .routes(utoipa_axum::routes!(list_content_revisions))
        .routes(utoipa_axum::routes!(add_content_revision))
        .routes(utoipa_axum::routes!(get_content_revision))
        .routes(utoipa_axum::routes!(list_content_destinations))
        .routes(utoipa_axum::routes!(publish_content))
        .routes(utoipa_axum::routes!(cancel_content_publishing))
        .routes(utoipa_axum::routes!(list_content_deliveries))
        .routes(utoipa_axum::routes!(list_content_publications))
        .routes(utoipa_axum::routes!(mark_content_published))
        .routes(utoipa_axum::routes!(list_destinations))
        .routes(utoipa_axum::routes!(create_destination))
        .routes(utoipa_axum::routes!(update_destination))
        .routes(utoipa_axum::routes!(remove_destination))
        .routes(utoipa_axum::routes!(configure_destination_github))
        .routes(utoipa_axum::routes!(create_visibility_run))
        .routes(utoipa_axum::routes!(list_visibility_runs))
        .routes(utoipa_axum::routes!(get_visibility_run))
        .routes(utoipa_axum::routes!(get_credits))
        .routes(utoipa_axum::routes!(list_actions))
        .routes(utoipa_axum::routes!(get_action))
        .routes(utoipa_axum::routes!(complete_action))
        .routes(utoipa_axum::routes!(start_action))
        .routes(utoipa_axum::routes!(report_action_sent))
        .routes(utoipa_axum::routes!(dismiss_action))
        .routes(utoipa_axum::routes!(restore_action))
        .routes(utoipa_axum::routes!(check_action_again))
        .routes(utoipa_axum::routes!(create_action_execution))
        .routes(utoipa_axum::routes!(complete_action_with_token))
        .routes(utoipa_axum::routes!(start_agent_connect))
        .routes(utoipa_axum::routes!(get_agent_connect))
        .routes(utoipa_axum::routes!(get_pricing))
        .routes(utoipa_axum::routes!(get_site_change))
        .routes(utoipa_axum::routes!(discard_site_change))
        .routes(utoipa_axum::routes!(create_site_change_pull_request))
        .routes(utoipa_axum::routes!(get_research))
        .routes(utoipa_axum::routes!(get_research_report))
        .routes(utoipa_axum::routes!(create_research_request))
        .routes(utoipa_axum::routes!(get_project))
        .routes(utoipa_axum::routes!(update_brand_competitor))
        .routes(utoipa_axum::routes!(remove_brand_competitor))
        .routes(utoipa_axum::routes!(get_billing))
        .routes(utoipa_axum::routes!(list_webhooks))
        .routes(utoipa_axum::routes!(create_webhook))
        .routes(utoipa_axum::routes!(update_webhook))
        .routes(utoipa_axum::routes!(delete_webhook))
        .routes(utoipa_axum::routes!(list_notification_rules))
        .routes(utoipa_axum::routes!(update_notification_rule))
        .routes(utoipa_axum::routes!(set_usage_cap))
        .routes(utoipa_axum::routes!(get_engines))
        .routes(utoipa_axum::routes!(put_engines))
}
