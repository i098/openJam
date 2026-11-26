use openjam_core::config::Config;
use openjam_publish::github::App;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    /// The GitHub App of this install. `None`: publishing to GitHub is off.
    pub github: Option<Arc<App>>,
}
