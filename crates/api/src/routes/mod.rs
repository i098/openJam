pub mod keys;
pub mod meta;
pub mod projects;
pub mod stubs;

use crate::{
    actions, analytics, answers, connect, content, notifications, opportunities, publishing,
    research, state::AppState, stream, webhooks,
};
use axum::Router;
use utoipa::openapi::{
    Info, Server,
    security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
};
use utoipa_axum::router::OpenApiRouter;

/// Every `/v1` route, with its contract. New routes go in a slice module, never in `stubs`.
pub fn build() -> (Router<AppState>, utoipa::openapi::OpenApi) {
    let builder: OpenApiRouter<AppState> = OpenApiRouter::new();
    let (router, mut openapi) = builder
        .routes(utoipa_axum::routes!(meta::get_me))
        .routes(utoipa_axum::routes!(meta::get_usage))
        .routes(utoipa_axum::routes!(
            projects::list_projects,
            projects::create_project
        ))
        .routes(utoipa_axum::routes!(
            keys::list_api_keys,
            keys::create_api_key
        ))
        .routes(utoipa_axum::routes!(keys::revoke_api_key))
        .routes(utoipa_axum::routes!(analytics::get_analytics_overview))
        .routes(utoipa_axum::routes!(analytics::list_prompt_analytics))
        .routes(utoipa_axum::routes!(analytics::list_analytics_metrics))
        .routes(utoipa_axum::routes!(analytics::list_keywords))
        .routes(utoipa_axum::routes!(answers::list_answers))
        .routes(utoipa_axum::routes!(answers::list_prompt_answers))
        // M6 agent surface.
        .routes(utoipa_axum::routes!(connect::start_agent_connect))
        .routes(utoipa_axum::routes!(connect::get_agent_connect))
        .routes(utoipa_axum::routes!(
            webhooks::list_webhooks,
            webhooks::create_webhook
        ))
        .routes(utoipa_axum::routes!(
            webhooks::update_webhook,
            webhooks::delete_webhook
        ))
        .routes(utoipa_axum::routes!(notifications::list_notification_rules))
        .routes(utoipa_axum::routes!(
            notifications::update_notification_rule
        ))
        .routes(utoipa_axum::routes!(notifications::list_notifications))
        .routes(utoipa_axum::routes!(notifications::mark_notifications_read))
        .routes(utoipa_axum::routes!(stream::stream_events))
        // M5 content and publishing.
        .routes(utoipa_axum::routes!(content::create_content))
        .routes(utoipa_axum::routes!(content::list_content))
        .routes(utoipa_axum::routes!(content::get_content))
        .routes(utoipa_axum::routes!(content::discard_content))
        .routes(utoipa_axum::routes!(content::restore_content))
        .routes(utoipa_axum::routes!(content::regenerate_content))
        .routes(utoipa_axum::routes!(content::request_content_edit))
        .routes(utoipa_axum::routes!(content::export_content_markdown))
        .routes(utoipa_axum::routes!(content::list_content_revisions))
        .routes(utoipa_axum::routes!(content::add_content_revision))
        .routes(utoipa_axum::routes!(content::get_content_revision))
        .routes(utoipa_axum::routes!(publishing::list_content_destinations))
        .routes(utoipa_axum::routes!(publishing::publish_content))
        .routes(utoipa_axum::routes!(publishing::cancel_content_publishing))
        .routes(utoipa_axum::routes!(publishing::list_content_deliveries))
        .routes(utoipa_axum::routes!(publishing::list_content_publications))
        .routes(utoipa_axum::routes!(publishing::mark_content_published))
        .routes(utoipa_axum::routes!(publishing::list_destinations))
        .routes(utoipa_axum::routes!(publishing::create_destination))
        .routes(utoipa_axum::routes!(publishing::update_destination))
        .routes(utoipa_axum::routes!(publishing::remove_destination))
        .routes(utoipa_axum::routes!(
            publishing::configure_destination_github
        ))
        // M4 research and actions.
        .routes(utoipa_axum::routes!(research::get_research))
        .routes(utoipa_axum::routes!(research::get_research_report))
        .routes(utoipa_axum::routes!(research::create_research_request))
        .routes(utoipa_axum::routes!(opportunities::list_opportunities))
        .routes(utoipa_axum::routes!(opportunities::get_opportunity))
        .routes(utoipa_axum::routes!(opportunities::dismiss_opportunity))
        .routes(utoipa_axum::routes!(opportunities::restore_opportunity))
        .routes(utoipa_axum::routes!(
            opportunities::request_opportunity_plan
        ))
        .routes(utoipa_axum::routes!(opportunities::get_opportunity_brief))
        .routes(utoipa_axum::routes!(opportunities::claim_opportunity))
        .routes(utoipa_axum::routes!(
            opportunities::release_opportunity_claim
        ))
        .routes(utoipa_axum::routes!(
            opportunities::report_opportunity_agent_failure
        ))
        .routes(utoipa_axum::routes!(actions::list_actions))
        .routes(utoipa_axum::routes!(actions::get_action))
        .routes(utoipa_axum::routes!(actions::start_action))
        .routes(utoipa_axum::routes!(actions::report_action_sent))
        .routes(utoipa_axum::routes!(actions::dismiss_action))
        .routes(utoipa_axum::routes!(actions::restore_action))
        .routes(utoipa_axum::routes!(actions::complete_action))
        .routes(utoipa_axum::routes!(actions::complete_action_with_token))
        .routes(utoipa_axum::routes!(actions::check_action_again))
        .routes(utoipa_axum::routes!(actions::create_action_execution))
        .merge(stubs::stub_routes())
        .split_for_parts();

    openapi.info = Info::new("openJam API", env!("CARGO_PKG_VERSION"));
    openapi.info.description = Some(
        "The `/v1` contract. Operations marked \"not built yet\" answer 501 until their slice lands."
            .into(),
    );
    openapi.servers = Some(vec![Server::new("/")]);
    openapi.components.as_mut().unwrap().add_security_scheme(
        "bearerAuth",
        SecurityScheme::Http(
            HttpBuilder::new()
                .scheme(HttpAuthScheme::Bearer)
                .bearer_format("sk")
                .build(),
        ),
    );
    (router, openapi)
}
