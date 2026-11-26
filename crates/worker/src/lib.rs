//! The job worker: claims Postgres jobs, runs scan calls and the judge, and starts scans on schedule.

mod content_update;
mod judge;
pub mod pages;
mod publish;
mod research;
mod scan;

use chrono::{DateTime, Duration, Utc};
use openjam_core::{
    actions::QUEUE_CHECK,
    jobs::{self, FailOutcome, Job},
    research::QUEUE_RESEARCH,
    tracking::{self, QUEUE_JUDGE, QUEUE_SCAN},
};
use openjam_engines::Router;
use openjam_publish::{delivery::QUEUE as QUEUE_PUBLISH, store::QUEUE_UPDATE};
use pages::PageReader;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

pub const MAINTENANCE: &str = "maintenance";
pub const WORKER_QUEUES: &[&str] = &[
    QUEUE_SCAN,
    QUEUE_JUDGE,
    QUEUE_PUBLISH,
    QUEUE_UPDATE,
    QUEUE_RESEARCH,
    QUEUE_CHECK,
    MAINTENANCE,
];
/// A job still `running` after this long lost its worker. Longer than the 120 s provider limit.
pub const STALE_SECS: i64 = 300;
/// Projects started in one scheduler tick.
const SCHEDULE_BATCH: i64 = 50;

/// Everything a job handler needs.
pub struct Services {
    pub pool: PgPool,
    pub router: Router,
    /// Scans a workspace may start per UTC day. `None`: no limit.
    pub scan_cap_per_day: Option<u32>,
    /// How research and action checks read web pages.
    pub pages: Arc<dyn PageReader>,
}

/// What a handler asks the loop to do with its job.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Run the job again at this time. It does not count as a failed attempt.
    RetryAt(DateTime<Utc>),
}

pub(crate) fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub(crate) fn payload_id(job: &Job, field: &str) -> Result<Uuid, String> {
    job.payload[field]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("job payload has no {field}"))
}

async fn handle(svc: &Services, job: &Job) -> Result<Outcome, String> {
    match job.queue.as_str() {
        QUEUE_SCAN => scan::handle(svc, job).await,
        QUEUE_JUDGE => judge::handle(svc, job).await,
        QUEUE_PUBLISH => publish::handle(svc, job).await,
        QUEUE_UPDATE => content_update::handle(svc, job).await,
        QUEUE_RESEARCH => research::handle_research(svc, job).await,
        QUEUE_CHECK => research::handle_check(svc, job).await,
        MAINTENANCE => {
            let freed = jobs::requeue_stale(&svc.pool, STALE_SECS)
                .await
                .map_err(text)?;
            if freed > 0 {
                tracing::warn!(freed, "requeued jobs from a lost worker");
            }
            Ok(Outcome::Done)
        }
        other => Err(format!("no handler for queue {other}")),
    }
}

/// Claim one due job and run it. `Ok(false)`: there was none.
pub async fn run_one(svc: &Services, worker_id: &str) -> Result<bool, String> {
    let Some(job) = jobs::claim(&svc.pool, WORKER_QUEUES, worker_id)
        .await
        .map_err(text)?
    else {
        return Ok(false);
    };
    match handle(svc, &job).await {
        Ok(Outcome::Done) => jobs::complete(&svc.pool, job.id).await.map_err(text)?,
        Ok(Outcome::RetryAt(at)) => {
            sqlx::query(
                "UPDATE jobs SET status = 'queued', run_at = $2, attempts = attempts - 1, locked_by = NULL, locked_at = NULL
                 WHERE id = $1",
            )
            .bind(job.id)
            .bind(at)
            .execute(&svc.pool)
            .await
            .map_err(text)?;
        }
        Err(error) => match jobs::fail(&svc.pool, &job, &error).await.map_err(text)? {
            FailOutcome::Retry { run_at } => {
                tracing::warn!(queue = %job.queue, %error, %run_at, "retrying")
            }
            FailOutcome::Dead => tracing::error!(queue = %job.queue, %error, "job is dead"),
        },
    }
    Ok(true)
}

/// Start the first research run of each project that has scanned. Returns the run ids.
pub async fn research_tick(svc: &Services) -> Result<Vec<Uuid>, String> {
    research::tick(svc).await
}

/// Start the scans that are due at `now`. Returns the run ids started.
pub async fn schedule_tick(svc: &Services, now: DateTime<Utc>) -> Result<Vec<Uuid>, String> {
    let mut started = Vec::new();
    for project in tracking::claim_due_projects(&svc.pool, now, SCHEDULE_BATCH)
        .await
        .map_err(text)?
    {
        match tracking::start_scan(&svc.pool, project, svc.scan_cap_per_day).await {
            Ok(Some(run)) => started.push(run),
            Ok(None) => tracing::info!(%project, "scan skipped: no prompts or platforms"),
            Err(openjam_core::Error::CapReached { cap }) => {
                // Try again after the cap resets, not on every tick.
                let tomorrow = (now + Duration::days(1))
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .expect("midnight")
                    .and_utc();
                tracking::postpone_project(&svc.pool, project, tomorrow)
                    .await
                    .map_err(text)?;
                tracing::warn!(%project, cap, "scan cap reached; scan moved to 00:00 UTC");
            }
            Err(error) => tracing::error!(%project, %error, "could not start scan"),
        }
    }
    Ok(started)
}
