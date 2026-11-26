//! Usage ledger and the optional scan cap.

use crate::{Error, Result};
use sqlx::PgPool;
use uuid::Uuid;

pub const KIND_SCAN: &str = "scan";

pub struct UsageEntry<'a> {
    pub workspace_id: Uuid,
    pub project_id: Option<Uuid>,
    /// `scan` for one scan, `judge`, `suggestion` and so on for other provider calls.
    pub kind: &'a str,
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    pub backend_id: Option<Uuid>,
    pub tokens_in: i32,
    pub tokens_out: i32,
    pub cost_cents: f64,
    pub reference: Option<&'a str>,
}

pub async fn record(pool: &PgPool, e: &UsageEntry<'_>) -> Result<()> {
    sqlx::query(
        "INSERT INTO usage_ledger
             (workspace_id, project_id, kind, provider, model, backend_id, tokens_in, tokens_out, cost_cents, ref)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::float8::numeric, $10)",
    )
    .bind(e.workspace_id)
    .bind(e.project_id)
    .bind(e.kind)
    .bind(e.provider)
    .bind(e.model)
    .bind(e.backend_id)
    .bind(e.tokens_in)
    .bind(e.tokens_out)
    .bind(e.cost_cents)
    .bind(e.reference)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn scans_today(pool: &PgPool, workspace_id: Uuid) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM usage_ledger
         WHERE workspace_id = $1 AND kind = $2 AND at >= date_trunc('day', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'",
    )
    .bind(workspace_id)
    .bind(KIND_SCAN)
    .fetch_one(pool)
    .await?)
}

/// Fail with `CapReached` when the workspace used its daily scans. No cap: always `Ok`.
pub async fn check_scan_cap(pool: &PgPool, workspace_id: Uuid, cap: Option<u32>) -> Result<()> {
    let Some(cap) = cap else { return Ok(()) };
    if scans_today(pool, workspace_id).await? >= i64::from(cap) {
        return Err(Error::CapReached { cap });
    }
    Ok(())
}
