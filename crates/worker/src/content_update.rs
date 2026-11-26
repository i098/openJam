//! Rewrite jobs: write an article again, or edit it as asked, and store the result as a revision.

use crate::{Outcome, Services, payload_id, text};
use chrono::{Duration, Utc};
use openjam_core::{
    jobs::Job,
    usage::{self, UsageEntry},
};
use openjam_engines::{EngineError, router::ModelTier};
use openjam_publish::{
    article::{Article, parse_reply, rewrite_prompt},
    store::{insert_revision, record_update_failure},
};
use sqlx::types::Json;
use uuid::Uuid;

/// Replies without a valid article that the job tolerates before it gives up.
const REPLY_TRIES: usize = 2;

pub async fn handle(svc: &Services, job: &Job) -> Result<Outcome, String> {
    let content_id = payload_id(job, "contentId")?;
    let basis = payload_id(job, "basisRevisionId")?;
    let instruction = job.payload["instruction"].as_str();

    let row: Option<(Uuid, Uuid, String, Json<Article>)> = sqlx::query_as(
        "SELECT p.workspace_id, p.id, c.state, r.article
         FROM contents c
         JOIN projects p ON p.id = c.project_id
         JOIN content_revisions r ON r.id = $2 AND r.content_id = c.id
         WHERE c.id = $1",
    )
    .bind(content_id)
    .bind(basis)
    .fetch_optional(&svc.pool)
    .await
    .map_err(text)?;
    let Some((workspace_id, project_id, state, Json(article))) = row else {
        return Ok(Outcome::Done);
    };
    if state != "drafted" {
        return Ok(Outcome::Done);
    }

    let (system, user) = rewrite_prompt(&article, instruction);
    let mut failure = (
        "agent_failed",
        "The model did not return a valid article.".to_owned(),
    );
    for _ in 0..REPLY_TRIES {
        let routed = match svc
            .router
            .complete(workspace_id, ModelTier::Big, &system, &user)
            .await
        {
            Ok(routed) => routed,
            Err(EngineError::RateLimited { retry_at, .. }) => {
                return Ok(Outcome::RetryAt(
                    retry_at.unwrap_or_else(|| Utc::now() + Duration::minutes(5)),
                ));
            }
            Err(e @ (EngineError::Timeout | EngineError::Unavailable(_))) => {
                let message = e.to_string();
                if job.attempts >= job.max_attempts {
                    record_update_failure(&svc.pool, content_id, "failed", &message)
                        .await
                        .map_err(text)?;
                }
                return Err(message);
            }
            Err(e @ EngineError::CapReached) => {
                failure = ("insufficient_credits", e.to_string());
                break;
            }
            Err(e) => {
                failure = ("agent_failed", e.to_string());
                break;
            }
        };
        let done = &routed.value;
        usage::record(
            &svc.pool,
            &UsageEntry {
                workspace_id,
                project_id: Some(project_id),
                kind: "content",
                provider: None,
                model: Some(&done.model),
                backend_id: None,
                tokens_in: done.tokens_in,
                tokens_out: done.tokens_out,
                cost_cents: done.cost_cents.unwrap_or(0.0),
                reference: Some(&content_id.to_string()),
            },
        )
        .await
        .map_err(text)?;
        if let Some(new) = parse_reply(&done.text) {
            store(svc, content_id, &new, basis).await?;
            return Ok(Outcome::Done);
        }
    }
    record_update_failure(&svc.pool, content_id, failure.0, &failure.1)
        .await
        .map_err(text)?;
    Ok(Outcome::Done)
}

async fn store(
    svc: &Services,
    content_id: Uuid,
    article: &Article,
    basis: Uuid,
) -> Result<(), String> {
    let mut tx = svc.pool.begin().await.map_err(text)?;
    sqlx::query("SELECT id FROM contents WHERE id = $1 FOR UPDATE")
        .bind(content_id)
        .execute(&mut *tx)
        .await
        .map_err(text)?;
    insert_revision(&mut tx, content_id, article, "pipeline", Some(basis), None)
        .await
        .map_err(text)?;
    tx.commit().await.map_err(text)
}
