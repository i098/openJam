pub mod keys;
pub mod meta;
pub mod projects;
pub mod stubs;

use crate::{analytics, answers, content, publishing, state::AppState};
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
