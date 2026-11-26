use tokio::signal::unix::{SignalKind, signal};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let (bind, cfg) = match openjam_runner::Config::from_env() {
        Ok(found) => found,
        Err(e) => {
            tracing::error!("{e}");
            std::process::exit(1);
        }
    };
    openjam_runner::sweep_temp_dirs();
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!("cannot bind {bind}: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!("runner listens on {bind}");
    let shutdown = async {
        // As PID 1 in a container the process must stop on SIGTERM by itself.
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    };
    if let Err(e) = axum::serve(listener, openjam_runner::router(cfg))
        .with_graceful_shutdown(shutdown)
        .await
    {
        tracing::error!("server failed: {e}");
        std::process::exit(1);
    }
}
