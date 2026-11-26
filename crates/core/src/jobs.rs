//! Postgres job queue: claim with `FOR UPDATE SKIP LOCKED`, retry with backoff, then `dead`.

use crate::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub struct Job {
    pub id: Uuid,
    pub queue: String,
    pub payload: Value,
    /// Starts at 1 on the first run. A claim counts as an attempt, so a crash still counts.
    pub attempts: i32,
    pub max_attempts: i32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FailOutcome {
    Retry { run_at: DateTime<Utc> },
    Dead,
}

/// Seconds to wait after the nth failed attempt: 5, 10, 20, 40 ... capped at one hour.
pub fn backoff_secs(attempts: i32) -> i64 {
    let exp = u32::try_from(attempts.saturating_sub(1))
        .unwrap_or(0)
        .min(10);
    (5_i64 << exp).min(3600)
}

/// Add a job. With a key, a second call with the same queue and key adds nothing and returns `None`.
pub async fn enqueue(
    pool: &PgPool,
    queue: &str,
    payload: &Value,
    idempotency_key: Option<&str>,
    run_at: Option<DateTime<Utc>>,
) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "INSERT INTO jobs (queue, payload, idempotency_key, run_at)
         VALUES ($1, $2, $3, COALESCE($4, now()))
         ON CONFLICT (queue, idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING
         RETURNING id",
    )
    .bind(queue)
    .bind(payload)
    .bind(idempotency_key)
    .bind(run_at)
    .fetch_optional(pool)
    .await?)
}

pub async fn claim(pool: &PgPool, queues: &[&str], worker_id: &str) -> Result<Option<Job>> {
    let queues: Vec<String> = queues.iter().map(|q| (*q).to_string()).collect();
    Ok(sqlx::query_as(
        "UPDATE jobs SET status = 'running', locked_by = $2, locked_at = now(), attempts = attempts + 1
         WHERE id = (
             SELECT id FROM jobs
             WHERE status = 'queued' AND run_at <= now() AND queue = ANY($1)
             ORDER BY run_at
             FOR UPDATE SKIP LOCKED
             LIMIT 1)
         RETURNING id, queue, payload, attempts, max_attempts",
    )
    .bind(&queues)
    .bind(worker_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn complete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE jobs SET status = 'done', finished_at = now(), locked_by = NULL, locked_at = NULL
         WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a failure. Retry with backoff until `max_attempts`, then mark the job `dead`.
pub async fn fail(pool: &PgPool, job: &Job, error: &str) -> Result<FailOutcome> {
    if job.attempts >= job.max_attempts {
        sqlx::query(
            "UPDATE jobs SET status = 'dead', last_error = $2, finished_at = now(),
                 locked_by = NULL, locked_at = NULL WHERE id = $1",
        )
        .bind(job.id)
        .bind(error)
        .execute(pool)
        .await?;
        return Ok(FailOutcome::Dead);
    }
    let run_at: DateTime<Utc> = sqlx::query_scalar(
        "UPDATE jobs SET status = 'queued', last_error = $2, locked_by = NULL, locked_at = NULL,
             run_at = now() + make_interval(secs => $3)
         WHERE id = $1 RETURNING run_at",
    )
    .bind(job.id)
    .bind(error)
    .bind(backoff_secs(job.attempts) as f64)
    .fetch_one(pool)
    .await?;
    Ok(FailOutcome::Retry { run_at })
}

/// Free jobs whose worker died: still `running` after `timeout_secs`. Out of attempts: `dead`.
pub async fn requeue_stale(pool: &PgPool, timeout_secs: i64) -> Result<u64> {
    Ok(sqlx::query(
        "UPDATE jobs SET
             status = CASE WHEN attempts >= max_attempts THEN 'dead' ELSE 'queued' END,
             last_error = 'worker lost', locked_by = NULL, locked_at = NULL,
             finished_at = CASE WHEN attempts >= max_attempts THEN now() END
         WHERE status = 'running' AND locked_at < now() - make_interval(secs => $1)",
    )
    .bind(timeout_secs as f64)
    .execute(pool)
    .await?
    .rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let steps: Vec<i64> = (1..=5).map(backoff_secs).collect();
        assert_eq!(steps, [5, 10, 20, 40, 80]);
        assert_eq!(backoff_secs(30), 3600);
        assert_eq!(backoff_secs(0), 5);
    }
}
