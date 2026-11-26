//! The job worker: claims Postgres jobs, retries with backoff, and keeps a minute tick.

use chrono::Utc;
use openjam_core::{
    config::Config,
    db,
    jobs::{self, FailOutcome},
};
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;

const MAINTENANCE: &str = "maintenance";
const WORKER_QUEUES: &[&str] = &["scan", MAINTENANCE];
/// A job still `running` after this long lost its worker.
const STALE_SECS: i64 = 120;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::from_env().map_err(|e| e.to_string())?;
    let pool = db::connect(&config.database_url)
        .await
        .map_err(|e| e.to_string())?;
    db::migrate(&pool).await.map_err(|e| e.to_string())?;

    let worker_id = format!("worker-{}", std::process::id());
    tracing::info!("{worker_id} watching {WORKER_QUEUES:?}");
    schedule_tick(&pool).await?;

    loop {
        match jobs::claim(&pool, WORKER_QUEUES, &worker_id).await {
            Ok(Some(job)) => {
                let result = run(&pool, &job.queue, &job.payload).await;
                match result {
                    Ok(()) => jobs::complete(&pool, job.id).await?,
                    Err(error) => match jobs::fail(&pool, &job, &error).await? {
                        FailOutcome::Retry { run_at } => {
                            tracing::warn!(queue = %job.queue, %error, %run_at, "retrying")
                        }
                        FailOutcome::Dead => {
                            tracing::error!(queue = %job.queue, %error, "job is dead")
                        }
                    },
                }
            }
            Ok(None) => tokio::time::sleep(Duration::from_millis(500)).await,
            Err(error) => {
                tracing::error!(%error, "claim failed");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// Queue the next maintenance tick: free stale jobs, then run again in one minute.
async fn schedule_tick(pool: &PgPool) -> Result<(), openjam_core::Error> {
    let key = format!("tick-{}", Utc::now().timestamp() / 60);
    jobs::enqueue(pool, MAINTENANCE, &json!({"op": "tick"}), Some(&key), None).await?;
    Ok(())
}

async fn run(pool: &PgPool, queue: &str, payload: &serde_json::Value) -> Result<(), String> {
    match queue {
        MAINTENANCE => {
            let freed = jobs::requeue_stale(pool, STALE_SECS)
                .await
                .map_err(|e| e.to_string())?;
            if freed > 0 {
                tracing::warn!(freed, "requeued jobs from a lost worker");
            }
            schedule_tick(pool).await.map_err(|e| e.to_string())?;
            Ok(())
        }
        other => Err(format!(
            "no handler for queue {other} in this build (payload {payload}); its slice adds one"
        )),
    }
}
