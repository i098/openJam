//! Deliveries: one request to send a content item to a destination, from schedule to live page.
//!
//! A GitHub delivery goes `scheduled` (waiting) -> `in_review` (a pull request is open) -> `live`
//! (the pull request was merged), or `failed` or `cancelled`. The worker polls the pull request, so
//! no webhook has to reach the install.

use crate::{
    article::{Article, file_path, public_url},
    github::{App, GithubError, Repo},
};
use chrono::{DateTime, Duration, Utc};
use openjam_core::{Error, Result, jobs};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// The job queue the worker reads. The payload is `{"deliveryId": ..., "withdraw"?: true}`.
pub const QUEUE: &str = "publish";
/// Branch prefix of every pull request openJam opens.
pub const BRANCH_PREFIX: &str = "jam-geo/";

/// Where a GitHub destination writes. Stored in `destinations.config`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubConfig {
    pub owner: String,
    pub name: String,
    pub base_branch: String,
    pub content_root: String,
    pub public_route_pattern: String,
    pub file_extension: String,
    pub installation_id: u64,
}

impl GithubConfig {
    pub fn from_value(config: &Value) -> Option<Self> {
        serde_json::from_value(config.clone()).ok()
    }
}

/// What the worker does next with a delivery job.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Done,
    RetryAt(DateTime<Utc>),
}

/// Schedule the current revision of `content_id` for `destination_id`. An earlier schedule of the
/// same pair moves to the new time. A pull request in review blocks it.
pub async fn schedule(
    pool: &PgPool,
    content_id: Uuid,
    destination_id: Uuid,
    deliver_at: DateTime<Utc>,
    automatic: bool,
) -> Result<Uuid> {
    let mut tx = pool.begin().await?;
    let in_review: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM deliveries
                        WHERE content_id = $1 AND destination_id = $2 AND status = 'in_review')",
    )
    .bind(content_id)
    .bind(destination_id)
    .fetch_one(&mut *tx)
    .await?;
    if in_review {
        return Err(Error::Conflict(
            "a pull request for this content is open; merge or withdraw it first".into(),
        ));
    }
    sqlx::query(
        "UPDATE deliveries SET status = 'cancelled', cancelled_at = now()
         WHERE content_id = $1 AND destination_id = $2 AND status = 'scheduled'",
    )
    .bind(content_id)
    .bind(destination_id)
    .execute(&mut *tx)
    .await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO deliveries (content_id, destination_id, revision_id, automatic, deliver_at)
         SELECT id, $2, current_revision_id, $3, $4 FROM contents WHERE id = $1
         RETURNING id",
    )
    .bind(content_id)
    .bind(destination_id)
    .bind(automatic)
    .bind(deliver_at)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    jobs::enqueue(
        pool,
        QUEUE,
        &json!({"deliveryId": id}),
        Some(&format!("deliver:{id}")),
        Some(deliver_at),
    )
    .await?;
    Ok(id)
}

/// Cancel the latest delivery of a pair. A pull request in review needs `withdraw`, and then the
/// worker closes it.
pub async fn cancel_latest(
    pool: &PgPool,
    content_id: Uuid,
    destination_id: Uuid,
    withdraw: bool,
) -> Result<()> {
    let latest: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, status FROM deliveries WHERE content_id = $1 AND destination_id = $2
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(content_id)
    .bind(destination_id)
    .fetch_optional(pool)
    .await?;
    let (id, status) = latest.ok_or(Error::NotFound("delivery"))?;
    match status.as_str() {
        "scheduled" => {
            cancel(pool, id).await?;
        }
        "in_review" if withdraw => {
            cancel(pool, id).await?;
            jobs::enqueue(
                pool,
                QUEUE,
                &json!({"deliveryId": id, "withdraw": true}),
                Some(&format!("withdraw:{id}")),
                None,
            )
            .await?;
        }
        "in_review" => {
            return Err(Error::Conflict(
                "a pull request is open; send withdraw=true to close it".into(),
            ));
        }
        "live" => {
            return Err(Error::Conflict(
                "the content is live; nothing to cancel".into(),
            ));
        }
        _ => {}
    }
    Ok(())
}

async fn cancel(pool: &PgPool, delivery_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE deliveries SET status = 'cancelled', cancelled_at = now()
         WHERE id = $1 AND status IN ('scheduled', 'in_review')",
    )
    .bind(delivery_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Cancel every open delivery of a destination, or only the automatic ones.
pub async fn cancel_destination(
    pool: &PgPool,
    destination_id: Uuid,
    automatic_only: bool,
) -> Result<()> {
    sqlx::query(
        "UPDATE deliveries SET status = 'cancelled', cancelled_at = now()
         WHERE destination_id = $1 AND status = 'scheduled' AND (automatic OR NOT $2)",
    )
    .bind(destination_id)
    .bind(automatic_only)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(FromRow)]
struct Row {
    status: String,
    content_id: Uuid,
    destination_id: Uuid,
    branch: Option<String>,
    pr_number: Option<i32>,
    attempted_at: Option<DateTime<Utc>>,
    config: Value,
    content_state: String,
    current_revision_id: Option<Uuid>,
    site_url: String,
}

/// Do the next step of a delivery. `Err` is a failure a retry may fix; the caller marks the
/// delivery failed with [`fail`] when the retries are used up.
pub async fn run(
    pool: &PgPool,
    app: Option<&App>,
    delivery_id: Uuid,
    withdraw: bool,
) -> Result<Step, String> {
    let row: Option<Row> = sqlx::query_as(
        "SELECT d.status, d.content_id, d.destination_id, d.branch, d.pr_number, d.attempted_at,
                dest.config, c.state AS content_state, c.current_revision_id, p.site_url
         FROM deliveries d
         JOIN destinations dest ON dest.id = d.destination_id
         JOIN contents c ON c.id = d.content_id
         JOIN projects p ON p.id = c.project_id
         WHERE d.id = $1",
    )
    .bind(delivery_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Ok(Step::Done);
    };
    let outcome = step(pool, app, delivery_id, withdraw, &row).await;
    match outcome {
        Err(e) if e.is_permanent() => {
            fail(pool, delivery_id, &e.to_string())
                .await
                .map_err(|e| e.to_string())?;
            Ok(Step::Done)
        }
        other => other.map_err(|e| e.to_string()),
    }
}

/// What can go wrong in one step: GitHub, or our own database.
enum StepError {
    Github(GithubError),
    Db(sqlx::Error),
    Data(String),
}

impl StepError {
    fn is_permanent(&self) -> bool {
        match self {
            Self::Github(e) => e.is_permanent(),
            Self::Data(_) => true,
            Self::Db(_) => false,
        }
    }
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Github(e) => e.fmt(f),
            Self::Db(e) => e.fmt(f),
            Self::Data(m) => f.write_str(m),
        }
    }
}

impl From<GithubError> for StepError {
    fn from(e: GithubError) -> Self {
        Self::Github(e)
    }
}

impl From<sqlx::Error> for StepError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

type StepResult = std::result::Result<Step, StepError>;

async fn step(pool: &PgPool, app: Option<&App>, id: Uuid, withdraw: bool, row: &Row) -> StepResult {
    match (row.status.as_str(), withdraw) {
        ("cancelled", true) => withdraw_pr(app, row).await,
        ("scheduled", _) => open_pr(pool, app, id, row).await,
        ("in_review", _) => sync_pr(pool, app, id, row).await,
        _ => Ok(Step::Done),
    }
}

async fn repo_for(
    app: Option<&App>,
    row: &Row,
) -> std::result::Result<(Repo, GithubConfig), StepError> {
    let config = GithubConfig::from_value(&row.config)
        .ok_or_else(|| StepError::Data("the destination has no GitHub repository".into()))?;
    let app = app.ok_or(GithubError::NotConfigured)?;
    let repo = app
        .repo(config.installation_id, &config.owner, &config.name)
        .await?;
    Ok((repo, config))
}

async fn withdraw_pr(app: Option<&App>, row: &Row) -> StepResult {
    if let (Some(number), Some(branch)) = (row.pr_number, &row.branch) {
        let (repo, _) = repo_for(app, row).await?;
        repo.withdraw(number, branch).await?;
    }
    Ok(Step::Done)
}

async fn open_pr(pool: &PgPool, app: Option<&App>, id: Uuid, row: &Row) -> StepResult {
    if row.content_state != "drafted" {
        cancel(pool, id)
            .await
            .map_err(|e| StepError::Data(e.to_string()))?;
        return Ok(Step::Done);
    }
    let revision = row
        .current_revision_id
        .ok_or_else(|| StepError::Data("the content has no revision".into()))?;
    let article: Value = sqlx::query_scalar("SELECT article FROM content_revisions WHERE id = $1")
        .bind(revision)
        .fetch_one(pool)
        .await?;
    let article: Article = serde_json::from_value(article)
        .map_err(|e| StepError::Data(format!("the stored article is not valid: {e}")))?;
    sqlx::query("UPDATE deliveries SET attempted_at = now(), revision_id = $2 WHERE id = $1")
        .bind(id)
        .bind(revision)
        .execute(pool)
        .await?;

    let (repo, config) = repo_for(app, row).await?;
    let base_sha = repo.branch_sha(&config.base_branch).await?.ok_or_else(|| {
        StepError::Data(format!(
            "the branch {} does not exist in {}/{}",
            config.base_branch, config.owner, config.name
        ))
    })?;
    let branch = row.branch.clone().unwrap_or_else(|| {
        format!(
            "{BRANCH_PREFIX}{}-{}",
            article.slug,
            &id.simple().to_string()[..8]
        )
    });
    repo.ensure_branch(&branch, &base_sha).await?;
    let path = file_path(&config.content_root, &article.slug, &config.file_extension);
    let file = article.to_markdown(Utc::now().date_naive());
    repo.put_file(&branch, &path, &file, &format!("Add {}", article.title))
        .await?;
    let body = format!(
        "openJam wrote this article for {}.\n\nIt adds `{path}`. Review it, then merge to publish.",
        row.site_url
    );
    let pr = repo
        .open_pr(
            &branch,
            &config.base_branch,
            &format!("Add article: {}", article.title),
            &body,
        )
        .await?;
    sqlx::query(
        "UPDATE deliveries SET status = 'in_review', pr_url = $2, pr_number = $3, branch = $4, error = NULL
         WHERE id = $1 AND status = 'scheduled'",
    )
    .bind(id)
    .bind(&pr.url)
    .bind(pr.number)
    .bind(&branch)
    .execute(pool)
    .await?;
    set_connection(pool, row.destination_id, None).await?;
    Ok(Step::RetryAt(Utc::now() + poll_after(Duration::zero())))
}

async fn sync_pr(pool: &PgPool, app: Option<&App>, id: Uuid, row: &Row) -> StepResult {
    let number = row
        .pr_number
        .ok_or_else(|| StepError::Data("the delivery has no pull request".into()))?;
    let (repo, config) = repo_for(app, row).await?;
    let pr = repo.pr(number).await?;
    if pr.open {
        let age = Utc::now() - row.attempted_at.unwrap_or_else(Utc::now);
        return Ok(Step::RetryAt(Utc::now() + poll_after(age)));
    }
    if !pr.merged {
        sqlx::query(
            "UPDATE deliveries SET status = 'cancelled', cancelled_at = now(),
                 error = 'The pull request was closed without merging.'
             WHERE id = $1 AND status = 'in_review'",
        )
        .bind(id)
        .execute(pool)
        .await?;
        return Ok(Step::Done);
    }
    let slug: String = sqlx::query_scalar(
        "SELECT r.article->>'slug' FROM deliveries d JOIN content_revisions r ON r.id = d.revision_id
         WHERE d.id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    let url = public_url(&config.public_route_pattern, &row.site_url, &slug);
    let publication: Uuid = sqlx::query_scalar(
        "INSERT INTO publications (content_id, delivery_id, url) VALUES ($1, $2, $3)
         ON CONFLICT (content_id, url) DO UPDATE SET delivery_id = EXCLUDED.delivery_id
         RETURNING id",
    )
    .bind(row.content_id)
    .bind(id)
    .bind(&url)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "UPDATE deliveries SET status = 'live', publication_id = $2, published_url = $3
         WHERE id = $1 AND status = 'in_review'",
    )
    .bind(id)
    .bind(publication)
    .bind(&url)
    .execute(pool)
    .await?;
    Ok(Step::Done)
}

/// How long to wait before the next look at an open pull request: quick at first, then slower.
fn poll_after(age: Duration) -> Duration {
    if age < Duration::hours(1) {
        Duration::minutes(2)
    } else if age < Duration::days(1) {
        Duration::minutes(10)
    } else {
        Duration::hours(1)
    }
}

/// Mark a delivery failed with a message, and the destination's connection with it.
pub async fn fail(pool: &PgPool, delivery_id: Uuid, message: &str) -> Result<()> {
    let destination: Option<Uuid> = sqlx::query_scalar(
        "UPDATE deliveries SET status = 'failed', error = $2, attempted_at = COALESCE(attempted_at, now())
         WHERE id = $1 AND status IN ('scheduled', 'in_review') RETURNING destination_id",
    )
    .bind(delivery_id)
    .bind(message)
    .fetch_optional(pool)
    .await?;
    if let Some(destination) = destination {
        set_connection(pool, destination, Some(message)).await?;
    }
    Ok(())
}

/// Record how the last call to the repository went: `None` is ok, a message is a problem.
pub async fn set_connection(
    pool: &PgPool,
    destination_id: Uuid,
    problem: Option<&str>,
) -> sqlx::Result<()> {
    let value = match problem {
        None => json!({"status": "ok"}),
        Some(message) => json!({"status": "problem", "message": message}),
    };
    sqlx::query("UPDATE destinations SET connection = $2 WHERE id = $1")
        .bind(destination_id)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

/// Schedule one drafted article per cadence period for every destination with auto-publish on.
/// Returns how many deliveries it scheduled.
pub async fn auto_publish_tick(pool: &PgPool, now: DateTime<Utc>) -> Result<usize> {
    let due: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "UPDATE destinations SET last_auto_at = $1
         WHERE auto_publish AND connection->>'status' = 'ok' AND config ? 'owner'
           AND (last_auto_at IS NULL OR last_auto_at <= $1 - CASE cadence
                    WHEN 'daily' THEN interval '1 day' ELSE interval '7 days' END)
         RETURNING id, project_id",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;
    let mut scheduled = 0;
    for (destination, project) in due {
        let next: Option<Uuid> = sqlx::query_scalar(
            "SELECT c.id FROM contents c JOIN content_revisions r ON r.id = c.current_revision_id
             WHERE c.project_id = $1 AND c.state = 'drafted' AND NOT r.needs_review
               AND NOT EXISTS (SELECT 1 FROM deliveries d WHERE d.content_id = c.id
                               AND d.destination_id = $2 AND d.status IN ('scheduled', 'in_review', 'live'))
               AND NOT EXISTS (SELECT 1 FROM publications p WHERE p.content_id = c.id)
             ORDER BY c.created_at LIMIT 1",
        )
        .bind(project)
        .bind(destination)
        .fetch_optional(pool)
        .await?;
        match next {
            Some(content) => {
                schedule(pool, content, destination, now, true).await?;
                scheduled += 1;
            }
            // Nothing to send: keep the period open for the next draft.
            None => {
                sqlx::query("UPDATE destinations SET last_auto_at = NULL WHERE id = $1")
                    .bind(destination)
                    .execute(pool)
                    .await?;
            }
        }
    }
    Ok(scheduled)
}
