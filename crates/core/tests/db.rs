//! Integration tests against a real Postgres. Set `DATABASE_URL`; each test gets its own database.

use openjam_core::{
    Error,
    jobs::{self, FailOutcome},
    usage::{self, UsageEntry},
};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
async fn job_retries_with_backoff_then_dies(pool: PgPool) {
    jobs::enqueue(&pool, "q", &json!({}), None, None)
        .await
        .unwrap();
    for attempt in 1..=5 {
        let job = jobs::claim(&pool, &["q"], "w1")
            .await
            .unwrap()
            .expect("job is due");
        assert_eq!(job.attempts, attempt);
        match jobs::fail(&pool, &job, "boom").await.unwrap() {
            FailOutcome::Retry { run_at } => {
                assert!(attempt < 5, "fifth failure must be dead");
                assert!(run_at > chrono::Utc::now(), "retry waits");
                sqlx::query("UPDATE jobs SET run_at = now()")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            FailOutcome::Dead => assert_eq!(attempt, 5),
        }
    }
    let status: String = sqlx::query_scalar("SELECT status FROM jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "dead");
    assert!(jobs::claim(&pool, &["q"], "w1").await.unwrap().is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn claim_is_exclusive_and_key_is_idempotent(pool: PgPool) {
    assert!(
        jobs::enqueue(&pool, "q", &json!({"n": 1}), Some("k"), None)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        jobs::enqueue(&pool, "q", &json!({"n": 2}), Some("k"), None)
            .await
            .unwrap()
            .is_none()
    );
    let job = jobs::claim(&pool, &["q"], "w1").await.unwrap().unwrap();
    assert!(jobs::claim(&pool, &["q"], "w2").await.unwrap().is_none());
    assert!(
        jobs::claim(&pool, &["other"], "w2")
            .await
            .unwrap()
            .is_none()
    );
    jobs::complete(&pool, job.id).await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
async fn stale_running_job_is_requeued(pool: PgPool) {
    jobs::enqueue(&pool, "q", &json!({}), None, None)
        .await
        .unwrap();
    jobs::claim(&pool, &["q"], "w1").await.unwrap().unwrap();
    assert_eq!(jobs::requeue_stale(&pool, 60).await.unwrap(), 0);
    sqlx::query("UPDATE jobs SET locked_at = now() - interval '2 minutes'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(jobs::requeue_stale(&pool, 60).await.unwrap(), 1);
    assert!(jobs::claim(&pool, &["q"], "w2").await.unwrap().is_some());
}

#[sqlx::test(migrations = "../../migrations")]
async fn scan_cap_is_off_by_default_and_counts_scans_only(pool: PgPool) {
    let ws: uuid::Uuid =
        sqlx::query_scalar("INSERT INTO workspaces (name) VALUES ('w') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    let entry = |kind| UsageEntry {
        workspace_id: ws,
        project_id: None,
        kind,
        provider: Some("openai"),
        model: None,
        backend_id: None,
        tokens_in: 10,
        tokens_out: 20,
        cost_cents: 0.5,
        reference: None,
    };
    for _ in 0..3 {
        usage::record(&pool, &entry("scan")).await.unwrap();
    }
    usage::record(&pool, &entry("judge")).await.unwrap();
    usage::check_scan_cap(&pool, ws, None).await.unwrap();
    usage::check_scan_cap(&pool, ws, Some(4)).await.unwrap();
    let err = usage::check_scan_cap(&pool, ws, Some(3)).await.unwrap_err();
    assert!(matches!(err, Error::CapReached { cap: 3 }));
}
