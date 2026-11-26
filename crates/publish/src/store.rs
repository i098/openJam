//! Revisions of a content item: one writer for the API and the worker.

use crate::article::Article;
use serde::Serialize;
use sqlx::{PgConnection, types::Json};
use uuid::Uuid;

/// Job queue of the worker that rewrites articles. The payload is
/// `{"contentId", "basisRevisionId", "instruction"}`; no instruction means write it again.
pub const QUEUE_UPDATE: &str = "content_update";

/// Store `article` as the next revision of `content_id` and make it the current draft. Call it
/// in a transaction that holds the content row lock, so two writers get different numbers.
pub async fn insert_revision(
    tx: &mut PgConnection,
    content_id: Uuid,
    article: &Article,
    source: &str,
    basis: Option<Uuid>,
    bundle: Option<Uuid>,
) -> sqlx::Result<Uuid> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO content_revisions
             (content_id, n, article, title, source, basis_revision_id, evidence_bundle_id)
         SELECT $1, COALESCE(max(n), 0) + 1, $2, $3, $4, $5, $6
         FROM content_revisions WHERE content_id = $1
         RETURNING id",
    )
    .bind(content_id)
    .bind(Json(article))
    .bind(&article.title)
    .bind(source)
    .bind(basis)
    .bind(bundle)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE contents SET current_revision_id = $2, update_failure = NULL WHERE id = $1",
    )
    .bind(content_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    Ok(id)
}

#[derive(Serialize)]
struct Failure<'a> {
    code: &'a str,
    message: &'a str,
    at: chrono::DateTime<chrono::Utc>,
}

/// Remember why the last rewrite failed. `code` is `insufficient_credits`, `agent_failed` or `failed`.
pub async fn record_update_failure(
    pool: &sqlx::PgPool,
    content_id: Uuid,
    code: &str,
    message: &str,
) -> sqlx::Result<()> {
    let failure = Failure {
        code,
        message,
        at: chrono::Utc::now(),
    };
    sqlx::query("UPDATE contents SET update_failure = $2 WHERE id = $1")
        .bind(content_id)
        .bind(Json(failure))
        .execute(pool)
        .await?;
    Ok(())
}
