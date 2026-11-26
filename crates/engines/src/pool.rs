//! Account choice and limit blocks.
//!
//! `rotate`: the next unblocked account, by owner priority then least recently used, so equal
//! accounts take turns and a limit moves the prompt to the next account. `standby`: the account
//! with the highest priority; a limit never moves the prompt to another account.

use crate::{Account, AccountMode, EngineError, Provider, Secret, secrets::Keyring};
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, Row, postgres::PgRow};
use std::sync::Arc;
use uuid::Uuid;

/// Block time after the first usage limit. It doubles for each repeat, up to level `MAX_LEVEL`.
const BASE_BLOCK_SECS: i64 = 60;
const MAX_LEVEL: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    Rotate,
    Standby,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    /// The whole account is out of plan use or credit.
    UsageLimit,
    /// A 429 that names no plan limit. It blocks one model.
    RateLimit,
    Unavailable,
}

impl BlockReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::UsageLimit => "usage_limit",
            Self::RateLimit => "rate_limit",
            Self::Unavailable => "unavailable",
        }
    }
}

pub enum Pick {
    Account(Account),
    /// Every candidate is blocked. The earliest end of a block.
    Blocked(DateTime<Utc>),
    /// No candidate: none exists, all are disabled, or all were tried.
    Empty,
}

/// Seconds to block after a usage limit at `level` (0 to 8): 60, 120, 240 ... 15360 (4 h 16 min).
pub fn block_secs(level: u8) -> i64 {
    BASE_BLOCK_SECS << level.min(MAX_LEVEL)
}

/// One event raises the level by one step at most: only a block that has ended counts as a repeat.
pub fn next_level(existing: Option<(DateTime<Utc>, u8)>, now: DateTime<Utc>) -> u8 {
    match existing {
        None => 0,
        Some((until, level)) if until <= now => (level + 1).min(MAX_LEVEL),
        Some((_, level)) => level,
    }
}

#[derive(Clone)]
pub struct Pool {
    db: PgPool,
    keyring: Arc<Keyring>,
}

impl Pool {
    pub fn new(db: PgPool, keyring: Arc<Keyring>) -> Self {
        Self { db, keyring }
    }

    pub fn db(&self) -> &PgPool {
        &self.db
    }

    pub fn keyring(&self) -> &Arc<Keyring> {
        &self.keyring
    }

    /// Choose an account of `mode` under `backend_id`, skipping the `tried` ones.
    /// `scope` is the model scope to check for a block, for example `model:gpt-5-mini`.
    pub async fn checkout(
        &self,
        backend_id: Uuid,
        provider: Provider,
        mode: AccountMode,
        rotation: Rotation,
        scope: &str,
        tried: &[Uuid],
    ) -> Result<Pick, EngineError> {
        let row = match rotation {
            Rotation::Rotate => {
                sqlx::query(
                    "UPDATE engine_accounts a SET last_used_at = now()
                     WHERE a.id = (
                         SELECT a2.id FROM engine_accounts a2
                         WHERE a2.backend_id = $1 AND a2.mode = $2 AND a2.enabled
                           AND a2.disabled_cause IS NULL AND a2.id <> ALL($3)
                           AND NOT EXISTS (SELECT 1 FROM engine_account_blocks b
                                           WHERE b.account_id = a2.id AND b.scope IN ('', $4)
                                             AND b.blocked_until > now())
                         ORDER BY a2.priority DESC, a2.last_used_at ASC NULLS FIRST, a2.id
                         FOR UPDATE OF a2 SKIP LOCKED
                         LIMIT 1)
                     RETURNING a.id, a.backend_id, a.mode, a.label, a.account_ref, a.credential_ciphertext",
                )
                .bind(backend_id)
                .bind(mode.as_str())
                .bind(tried)
                .bind(scope)
                .fetch_optional(&self.db)
                .await?
            }
            Rotation::Standby => {
                let row = sqlx::query(
                    "SELECT a.id, a.backend_id, a.mode, a.label, a.account_ref, a.credential_ciphertext,
                            (SELECT max(b.blocked_until) FROM engine_account_blocks b
                             WHERE b.account_id = a.id AND b.scope IN ('', $4)
                               AND b.blocked_until > now()) AS blocked_until
                     FROM engine_accounts a
                     WHERE a.backend_id = $1 AND a.mode = $2 AND a.enabled
                       AND a.disabled_cause IS NULL AND a.id <> ALL($3)
                     ORDER BY a.priority DESC, a.created_at, a.id
                     LIMIT 1",
                )
                .bind(backend_id)
                .bind(mode.as_str())
                .bind(tried)
                .bind(scope)
                .fetch_optional(&self.db)
                .await?;
                if let Some(until) = row.as_ref().and_then(|r| r.get::<Option<DateTime<Utc>>, _>("blocked_until")) {
                    return Ok(Pick::Blocked(until));
                }
                if let Some(row) = &row {
                    sqlx::query("UPDATE engine_accounts SET last_used_at = now() WHERE id = $1")
                        .bind(row.get::<Uuid, _>("id"))
                        .execute(&self.db)
                        .await?;
                }
                row
            }
        };
        match row {
            Some(row) => Ok(Pick::Account(self.account(&row, provider, mode)?)),
            None => {
                let until: Option<DateTime<Utc>> = sqlx::query_scalar(
                    "SELECT min(b.blocked_until)
                     FROM engine_accounts a JOIN engine_account_blocks b ON b.account_id = a.id
                     WHERE a.backend_id = $1 AND a.mode = $2 AND a.enabled
                       AND a.disabled_cause IS NULL AND a.id <> ALL($3)
                       AND b.scope IN ('', $4) AND b.blocked_until > now()",
                )
                .bind(backend_id)
                .bind(mode.as_str())
                .bind(tried)
                .bind(scope)
                .fetch_one(&self.db)
                .await?;
                Ok(until.map_or(Pick::Empty, Pick::Blocked))
            }
        }
    }

    fn account(
        &self,
        row: &PgRow,
        provider: Provider,
        mode: AccountMode,
    ) -> Result<Account, EngineError> {
        let id: Uuid = row.get("id");
        // A plan account reads its own tokens; only an API key goes to the engine.
        let secret = match (mode, row.get::<Option<Vec<u8>>, _>("credential_ciphertext")) {
            (AccountMode::Key, Some(blob)) => Some(
                self.keyring
                    .open_string(id, &blob)
                    .map_err(|e| EngineError::Unavailable(format!("account {id}: {e}")))?,
            ),
            _ => None,
        };
        Ok(Account {
            id,
            backend_id: row.get("backend_id"),
            provider,
            mode,
            label: row.get("label"),
            account_ref: row.get("account_ref"),
            secret,
        })
    }

    /// Block `scope` of an account after a limit. A usage limit raises the back-off level by one
    /// step per ended block. `retry_at` from the provider wins over the computed time.
    pub async fn block(
        &self,
        account_id: Uuid,
        scope: &str,
        retry_at: Option<DateTime<Utc>>,
        reason: BlockReason,
    ) -> Result<DateTime<Utc>, EngineError> {
        let mut tx = self.db.begin().await?;
        let existing: Option<(DateTime<Utc>, i16)> = sqlx::query_as(
            "SELECT blocked_until, backoff_level FROM engine_account_blocks
             WHERE account_id = $1 AND scope = $2 FOR UPDATE",
        )
        .bind(account_id)
        .bind(scope)
        .fetch_optional(&mut *tx)
        .await?;
        let now = Utc::now();
        let level = match reason {
            BlockReason::UsageLimit => next_level(
                existing.map(|(until, level)| (until, u8::try_from(level).unwrap_or(0))),
                now,
            ),
            _ => 0,
        };
        let until = retry_at.unwrap_or_else(|| {
            let secs = match reason {
                BlockReason::UsageLimit => block_secs(level),
                _ => BASE_BLOCK_SECS,
            };
            now + Duration::seconds(secs)
        });
        let until: DateTime<Utc> = sqlx::query_scalar(
            "INSERT INTO engine_account_blocks (account_id, scope, blocked_until, backoff_level, reason)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (account_id, scope) DO UPDATE SET
                 blocked_until = GREATEST(engine_account_blocks.blocked_until, excluded.blocked_until),
                 backoff_level = excluded.backoff_level, reason = excluded.reason, updated_at = now()
             RETURNING blocked_until",
        )
        .bind(account_id)
        .bind(scope)
        .bind(until)
        .bind(i16::from(level))
        .bind(reason.as_str())
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(until)
    }

    /// A success ends every block of the account and starts the back-off again at level 0.
    pub async fn report_ok(&self, account_id: Uuid) -> Result<(), EngineError> {
        sqlx::query("DELETE FROM engine_account_blocks WHERE account_id = $1")
            .bind(account_id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    /// Stop using an account until the owner fixes it. `cause` is shown to the owner.
    pub async fn disable(&self, account_id: Uuid, cause: &str) -> Result<(), EngineError> {
        sqlx::query("UPDATE engine_accounts SET disabled_cause = $2 WHERE id = $1")
            .bind(account_id)
            .bind(cause)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    /// The backend row for a workspace and provider. Made when missing. Gemini starts off.
    pub async fn ensure_backend(
        &self,
        workspace_id: Uuid,
        provider: Provider,
    ) -> Result<Uuid, sqlx::Error> {
        sqlx::query_scalar(
            "INSERT INTO engine_backends (workspace_id, provider, enabled) VALUES ($1, $2, $3)
             ON CONFLICT (workspace_id, provider) DO UPDATE SET provider = excluded.provider
             RETURNING id",
        )
        .bind(workspace_id)
        .bind(provider.as_str())
        .bind(provider != Provider::Gemini)
        .fetch_one(&self.db)
        .await
    }

    /// Store an API key as an account. The key is sealed; the row id is part of the seal.
    pub async fn add_key(
        &self,
        workspace_id: Uuid,
        provider: Provider,
        label: &str,
        key: &Secret,
    ) -> Result<Uuid, sqlx::Error> {
        let backend = self.ensure_backend(workspace_id, provider).await?;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO engine_accounts (id, backend_id, mode, label, credential_ciphertext)
             VALUES ($1, $2, 'key', $3, $4)",
        )
        .bind(id)
        .bind(backend)
        .bind(label)
        .bind(self.keyring.seal(id, key.expose().as_bytes()))
        .execute(&self.db)
        .await?;
        Ok(id)
    }

    /// Record a Claude login that lives in the runner under `folder`. openJam holds no secret.
    pub async fn add_claude_plan(
        &self,
        workspace_id: Uuid,
        label: &str,
        folder: &str,
    ) -> Result<Uuid, sqlx::Error> {
        let backend = self.ensure_backend(workspace_id, Provider::Claude).await?;
        sqlx::query_scalar(
            "INSERT INTO engine_accounts (backend_id, mode, label, account_ref)
             VALUES ($1, 'plan', $2, $3) RETURNING id",
        )
        .bind(backend)
        .bind(label)
        .bind(folder)
        .fetch_one(&self.db)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_time_doubles_from_one_minute_up_to_level_eight() {
        let secs: Vec<i64> = (0..=8).map(block_secs).collect();
        assert_eq!(secs[..4], [60, 120, 240, 480]);
        assert_eq!(secs[8], 15360, "level 8 is the top");
        assert_eq!(block_secs(200), 15360);
    }

    #[test]
    fn one_event_raises_the_level_one_step_and_only_after_a_block_ended() {
        let now = Utc::now();
        assert_eq!(next_level(None, now), 0);
        assert_eq!(next_level(Some((now - Duration::seconds(1), 2)), now), 3);
        assert_eq!(
            next_level(Some((now + Duration::seconds(30), 2)), now),
            2,
            "still blocked: keep"
        );
        assert_eq!(
            next_level(Some((now - Duration::seconds(1), 8)), now),
            8,
            "level is capped"
        );
    }
}
