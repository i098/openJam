use openjam_api::state::AppState;
use openjam_api::{openapi_yaml, router};
use openjam_core::{config::Config, db};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // One JSON object per line by default; OPENJAM_LOG_FORMAT=text gives readable lines.
    let logs = tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
    );
    if std::env::var("OPENJAM_LOG_FORMAT").is_ok_and(|f| f == "text") {
        logs.init();
    } else {
        logs.json().init();
    }

    let config = Config::from_env().map_err(|e| e.to_string())?;
    let pool = db::connect(&config.database_url)
        .await
        .map_err(|e| e.to_string())?;
    db::migrate(&pool).await.map_err(|e| e.to_string())?;

    // Fans events out to the inbox and webhooks, and sends the webhook deliveries.
    openjam_api::dispatch::spawn(pool.clone());
    let bind = config.bind.clone();
    let state = AppState {
        pool,
        config: Arc::new(config),
    };
    // Fail here rather than on the first request if the contract cannot be built.
    let _ = openapi_yaml();
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("openJam API on {bind}");
    axum::serve(listener, router(state)).await?;
    Ok(())
}
