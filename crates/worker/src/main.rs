//! The worker binary: job loop, minute maintenance, the scan scheduler and plan-token refresh.

use openjam_core::{config::Config, db};
use openjam_engines::{
    Engine, Provider, Router, Secret,
    claude::ClaudePlanEngine,
    key::KeyEngine,
    pool::Pool,
    router::JudgeModels,
    secrets::Keyring,
    siwc::{OpenaiPlanEngine, Siwc, SiwcConfig},
};
use openjam_worker::{STALE_SECS, Services, WORKER_QUEUES, run_one, schedule_tick};
use std::{path::Path, sync::Arc, time::Duration};

const DEFAULT_KEY_FILE: &str = "/var/lib/openjam/master.key";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::from_env().map_err(|e| e.to_string())?;
    let db = db::connect(&config.database_url)
        .await
        .map_err(|e| e.to_string())?;
    db::migrate(&db).await.map_err(|e| e.to_string())?;

    let key_file =
        std::env::var("OPENJAM_MASTER_KEY_FILE").unwrap_or_else(|_| DEFAULT_KEY_FILE.into());
    let keyring = Arc::new(
        Keyring::load(&db, Path::new(&key_file))
            .await
            .map_err(|e| e.to_string())?,
    );
    let pool = Pool::new(db.clone(), keyring);
    let siwc = Arc::new(Siwc::new(pool.clone(), SiwcConfig::from_env()));

    let mut engines: Vec<Arc<dyn Engine>> = [
        Provider::Openai,
        Provider::Claude,
        Provider::Perplexity,
        Provider::Gemini,
    ]
    .into_iter()
    .map(|p| Arc::new(KeyEngine::new(p, None)) as Arc<dyn Engine>)
    .collect();
    engines.push(Arc::new(OpenaiPlanEngine::new(siwc.clone())));
    match (env("OPENJAM_RUNNER_URL"), env("RUNNER_SHARED_SECRET")) {
        (Some(url), Some(secret)) => {
            engines.push(Arc::new(ClaudePlanEngine::new(&url, Secret::new(secret))))
        }
        _ => tracing::info!("OPENJAM_RUNNER_URL is not set: Claude plan accounts are off"),
    }

    let svc = Arc::new(Services {
        pool: db.clone(),
        router: Router::new(pool, engines, JudgeModels::from_env()),
        scan_cap_per_day: config.scan_cap_per_day,
    });

    let worker_id = format!("worker-{}", std::process::id());
    tracing::info!("{worker_id} watching {WORKER_QUEUES:?}");
    spawn_every(Duration::from_secs(30), svc.clone(), |svc| async move {
        match schedule_tick(&svc, chrono::Utc::now()).await {
            Ok(started) if !started.is_empty() => {
                tracing::info!(runs = started.len(), "scans started")
            }
            Ok(_) => {}
            Err(e) => tracing::error!(%e, "scheduler tick failed"),
        }
    });
    spawn_every(Duration::from_secs(60), svc.clone(), move |svc| {
        let siwc = siwc.clone();
        async move {
            if let Err(e) = openjam_core::jobs::requeue_stale(&svc.pool, STALE_SECS).await {
                tracing::error!(%e, "requeue failed");
            }
            if let Err(e) = siwc.refresh_due().await {
                tracing::error!(%e, "token refresh failed");
            }
        }
    });

    loop {
        match run_one(&svc, &worker_id).await {
            Ok(true) => {}
            Ok(false) => tokio::time::sleep(Duration::from_millis(500)).await,
            Err(error) => {
                tracing::error!(%error, "job loop failed");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn spawn_every<F, Fut>(period: Duration, svc: Arc<Services>, task: F)
where
    F: Fn(Arc<Services>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(period);
        loop {
            tick.tick().await;
            task(svc.clone()).await;
        }
    });
}
