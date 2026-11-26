//! Tracking data: prompts, the cadence scheduler, scan runs and calls, stored answers.
//!
//! Provider calls are not here. The worker makes them through the engines crate and stores the
//! results with these functions.

use crate::{
    Error, Result, jobs,
    tracking_parse::{BrandSpec, Verdict, cites_domain, domain_of},
    usage::{self, UsageEntry},
};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::str::FromStr;
use uuid::Uuid;

pub const QUEUE_SCAN: &str = "scan";
pub const QUEUE_JUDGE: &str = "judge";

// ---- cadence --------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cadence {
    Daily,
    EveryOtherDay,
    Weekly,
}

impl Cadence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::EveryOtherDay => "every_other_day",
            Self::Weekly => "weekly",
        }
    }

    pub fn interval(self) -> Duration {
        match self {
            Self::Daily => Duration::days(1),
            Self::EveryOtherDay => Duration::days(2),
            Self::Weekly => Duration::days(7),
        }
    }

    /// The next scan time. It counts from the planned time, so a late tick does not shift the
    /// schedule; a project that is more than one interval behind starts again from `now`.
    pub fn next_after(self, planned: DateTime<Utc>, now: DateTime<Utc>) -> DateTime<Utc> {
        let next = planned + self.interval();
        if next > now {
            next
        } else {
            now + self.interval()
        }
    }
}

impl FromStr for Cadence {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "daily" => Ok(Self::Daily),
            "every_other_day" => Ok(Self::EveryOtherDay),
            "weekly" => Ok(Self::Weekly),
            other => Err(format!("unknown cadence {other}")),
        }
    }
}

/// Take the projects whose scan is due and move each to its next time in the same statement, so two
/// workers never start the same scan. Returns the project ids.
pub async fn claim_due_projects(
    pool: &PgPool,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<Uuid>> {
    let mut tx = pool.begin().await?;
    let due = sqlx::query(
        "SELECT id, cadence, next_scan_at FROM projects
         WHERE status = 'active' AND archived_at IS NULL AND next_scan_at <= $1
         ORDER BY next_scan_at LIMIT $2 FOR UPDATE SKIP LOCKED",
    )
    .bind(now)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    let mut ids = Vec::with_capacity(due.len());
    for row in due {
        let (id, planned): (Uuid, DateTime<Utc>) = (row.get("id"), row.get("next_scan_at"));
        let cadence = row
            .get::<String, _>("cadence")
            .parse::<Cadence>()
            .map_err(Error::Internal)?;
        sqlx::query("UPDATE projects SET next_scan_at = $2 WHERE id = $1")
            .bind(id)
            .bind(cadence.next_after(planned, now))
            .execute(&mut *tx)
            .await?;
        ids.push(id);
    }
    tx.commit().await?;
    Ok(ids)
}

/// Move a project's next scan to `at` (for example, to the next UTC midnight after the cap).
pub async fn postpone_project(pool: &PgPool, project_id: Uuid, at: DateTime<Utc>) -> Result<()> {
    sqlx::query("UPDATE projects SET next_scan_at = $2 WHERE id = $1")
        .bind(project_id)
        .bind(at)
        .execute(pool)
        .await?;
    Ok(())
}

// ---- prompts --------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct NewPrompt {
    pub text: String,
    pub topic: Option<String>,
    pub persona: Option<String>,
    pub intent: Option<String>,
    pub branded: bool,
}

pub async fn add_prompt(pool: &PgPool, project_id: Uuid, p: &NewPrompt) -> Result<Uuid> {
    let text = p.text.trim();
    if text.is_empty() {
        return Err(Error::InvalidRequest("a prompt needs text".into()));
    }
    Ok(sqlx::query_scalar(
        "INSERT INTO prompts (project_id, text, topic, persona, intent, branded)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(project_id)
    .bind(text)
    .bind(&p.topic)
    .bind(&p.persona)
    .bind(&p.intent)
    .bind(p.branded)
    .fetch_one(pool)
    .await?)
}

// ---- scan runs ------------------------------------------------------------------------------

/// Start one scan: every tracked, active prompt on every platform of the project. Returns the run
/// id, or `None` when the project has no prompt or no platform. Fails with `CapReached` over the
/// daily cap. Each call is a job on the `scan` queue.
pub async fn start_scan(
    pool: &PgPool,
    project_id: Uuid,
    cap_per_day: Option<u32>,
) -> Result<Option<Uuid>> {
    let project = sqlx::query(
        "SELECT workspace_id, platforms FROM projects WHERE id = $1 AND archived_at IS NULL",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?
    .ok_or(Error::NotFound("project"))?;
    let workspace_id: Uuid = project.get("workspace_id");
    let platforms: Vec<String> = project.get("platforms");
    let prompts: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM prompts WHERE project_id = $1 AND list = 'tracked' AND status = 'active' ORDER BY created_at, id",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    if prompts.is_empty() || platforms.is_empty() {
        return Ok(None);
    }
    usage::check_scan_cap(pool, workspace_id, cap_per_day).await?;

    let mut tx = pool.begin().await?;
    let run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO scan_runs (project_id, status, prompt_count, platform_count, started_at)
         VALUES ($1, 'running', $2, $3, now()) RETURNING id",
    )
    .bind(project_id)
    .bind(prompts.len() as i32)
    .bind(platforms.len() as i32)
    .fetch_one(&mut *tx)
    .await?;
    let mut calls = Vec::with_capacity(prompts.len() * platforms.len());
    for prompt in &prompts {
        for platform in &platforms {
            let call: Uuid = sqlx::query_scalar(
                "INSERT INTO scan_calls (run_id, prompt_id, platform, backend_id)
                 VALUES ($1, $2, $3, (SELECT id FROM engine_backends WHERE workspace_id = $4 AND provider = $3))
                 RETURNING id",
            )
            .bind(run_id)
            .bind(prompt)
            .bind(platform)
            .bind(workspace_id)
            .fetch_one(&mut *tx)
            .await?;
            calls.push(call);
        }
    }
    sqlx::query("UPDATE projects SET last_scan_at = now(), first_scan_at = COALESCE(first_scan_at, now()) WHERE id = $1")
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    usage::record(
        pool,
        &UsageEntry {
            workspace_id,
            project_id: Some(project_id),
            kind: usage::KIND_SCAN,
            provider: None,
            model: None,
            backend_id: None,
            tokens_in: 0,
            tokens_out: 0,
            cost_cents: 0.0,
            reference: Some(&run_id.to_string()),
        },
    )
    .await?;
    for call in calls {
        jobs::enqueue(
            pool,
            QUEUE_SCAN,
            &json!({ "call_id": call }),
            Some(&call.to_string()),
            None,
        )
        .await?;
    }
    Ok(Some(run_id))
}

/// Everything the worker needs to run one call.
#[derive(Clone, Debug)]
pub struct CallContext {
    pub call_id: Uuid,
    pub run_id: Uuid,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub platform: String,
    pub prompt_id: Uuid,
    pub prompt: String,
    pub project_id: Uuid,
    pub workspace_id: Uuid,
    pub locale: String,
}

pub async fn load_call(pool: &PgPool, call_id: Uuid) -> Result<Option<CallContext>> {
    let row = sqlx::query(
        "SELECT c.id, c.run_id, c.status, c.created_at, c.platform, c.prompt_id, pr.text, p.id AS project_id,
                p.workspace_id, p.locale
         FROM scan_calls c
         JOIN prompts pr ON pr.id = c.prompt_id
         JOIN scan_runs r ON r.id = c.run_id
         JOIN projects p ON p.id = r.project_id
         WHERE c.id = $1",
    )
    .bind(call_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| CallContext {
        call_id: r.get("id"),
        run_id: r.get("run_id"),
        status: r.get("status"),
        created_at: r.get("created_at"),
        platform: r.get("platform"),
        prompt_id: r.get("prompt_id"),
        prompt: r.get("text"),
        project_id: r.get("project_id"),
        workspace_id: r.get("workspace_id"),
        locale: r.get("locale"),
    }))
}

pub fn is_finished(status: &str) -> bool {
    matches!(status, "done" | "failed")
}

/// Mark a call as started: `running`, one more attempt.
pub async fn begin_call(pool: &PgPool, call_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE scan_calls SET status = 'running', attempts = attempts + 1, run_after = NULL WHERE id = $1")
        .bind(call_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// The call waits for a plan limit to end.
pub async fn wait_call(
    pool: &PgPool,
    call_id: Uuid,
    until: DateTime<Utc>,
    code: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE scan_calls SET status = 'waiting', run_after = $2, error_code = $3 WHERE id = $1",
    )
    .bind(call_id)
    .bind(until)
    .bind(code)
    .execute(pool)
    .await?;
    Ok(())
}

/// End a call with an error and update its run.
pub async fn fail_call(pool: &PgPool, call: &CallContext, code: &str, message: &str) -> Result<()> {
    sqlx::query("UPDATE scan_calls SET status = 'failed', error_code = $2, error = $3, finished_at = now() WHERE id = $1")
        .bind(call.call_id)
        .bind(code)
        .bind(message)
        .execute(pool)
        .await?;
    refresh_run(pool, call.run_id).await
}

/// A provider answer ready to store.
pub struct NewAnswer<'a> {
    pub text: &'a str,
    pub citations: &'a [(String, Option<String>)],
    pub route: &'a str,
    pub model: &'a str,
    pub account_id: Option<Uuid>,
    pub raw: &'a Value,
    pub cost_cents: f64,
}

/// Store the answer of a call, its citations and its cost; end the call and update its run.
/// The answer starts unassessed. Returns its id. A second call with the same call id returns the first answer.
pub async fn record_answer(
    pool: &PgPool,
    call: &CallContext,
    own: &BrandSpec,
    a: &NewAnswer<'_>,
) -> Result<Uuid> {
    let mut tx = pool.begin().await?;
    let answer_id: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO answers (run_id, prompt_id, platform, text, own_domain_cited, route, model, account_id, call_id, raw)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (call_id) WHERE call_id IS NOT NULL DO NOTHING RETURNING id",
    )
    .bind(call.run_id)
    .bind(call.prompt_id)
    .bind(&call.platform)
    .bind(a.text)
    .bind(cites_domain(&a.citations.iter().map(|(u, _)| u.as_str()).collect::<Vec<_>>(), own))
    .bind(a.route)
    .bind(a.model)
    .bind(a.account_id)
    .bind(call.call_id)
    .bind(a.raw)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(answer_id) = answer_id else {
        tx.rollback().await?;
        return Ok(
            sqlx::query_scalar("SELECT id FROM answers WHERE call_id = $1")
                .bind(call.call_id)
                .fetch_one(pool)
                .await?,
        );
    };
    for (url, title) in a.citations {
        sqlx::query(
            "INSERT INTO answer_citations (answer_id, url, domain, title) VALUES ($1, $2, $3, $4)",
        )
        .bind(answer_id)
        .bind(url)
        .bind(domain_of(url).unwrap_or_default())
        .bind(title)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE scan_calls SET status = 'done', error = NULL, error_code = NULL, finished_at = now() WHERE id = $1")
        .bind(call.call_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE scan_runs SET cost_cents = cost_cents + $2::float8::numeric WHERE id = $1")
        .bind(call.run_id)
        .bind(a.cost_cents)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    refresh_run(pool, call.run_id).await?;
    jobs::enqueue(
        pool,
        QUEUE_JUDGE,
        &json!({ "answer_id": answer_id }),
        Some(&answer_id.to_string()),
        None,
    )
    .await?;
    Ok(answer_id)
}

/// Set the run status from its calls: `running` while any call is open, then `complete`,
/// `partial`, `cap_reached` or `failed`.
pub async fn refresh_run(pool: &PgPool, run_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE scan_runs r SET
             status = s.status,
             finished_at = CASE WHEN s.status = 'running' THEN NULL ELSE COALESCE(r.finished_at, now()) END
         FROM (SELECT CASE
                   WHEN count(*) FILTER (WHERE status NOT IN ('done', 'failed')) > 0 THEN 'running'
                   WHEN count(*) FILTER (WHERE status = 'failed') = 0 THEN 'complete'
                   WHEN count(*) FILTER (WHERE error_code = 'cap_reached') > 0 THEN 'cap_reached'
                   WHEN count(*) FILTER (WHERE status = 'done') > 0 THEN 'partial'
                   ELSE 'failed' END AS status
               FROM scan_calls WHERE run_id = $1) s
         WHERE r.id = $1",
    )
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ---- brands and judging ---------------------------------------------------------------------

pub async fn own_brand(pool: &PgPool, project_id: Uuid) -> Result<BrandSpec> {
    let row = sqlx::query("SELECT name, domain FROM projects WHERE id = $1")
        .bind(project_id)
        .fetch_optional(pool)
        .await?;
    let row = row.ok_or(Error::NotFound("project"))?;
    Ok(BrandSpec {
        name: row.get("name"),
        domain: row.get("domain"),
        aliases: vec![],
    })
}

pub async fn competitors(pool: &PgPool, project_id: Uuid) -> Result<Vec<BrandSpec>> {
    let rows = sqlx::query(
        "SELECT name, domain, aliases FROM competitors WHERE project_id = $1 ORDER BY name",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| BrandSpec {
            name: r.get("name"),
            domain: r.get("domain"),
            aliases: r.get("aliases"),
        })
        .collect())
}

/// What the judge needs to read one answer.
pub struct AnswerContext {
    pub answer_id: Uuid,
    pub workspace_id: Uuid,
    pub project_id: Uuid,
    pub question: String,
    pub text: String,
    pub assessed: bool,
}

pub async fn load_answer(pool: &PgPool, answer_id: Uuid) -> Result<Option<AnswerContext>> {
    let row = sqlx::query(
        "SELECT a.id, a.text, a.assessed, pr.text AS question, p.id AS project_id, p.workspace_id
         FROM answers a JOIN prompts pr ON pr.id = a.prompt_id JOIN projects p ON p.id = pr.project_id
         WHERE a.id = $1",
    )
    .bind(answer_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| AnswerContext {
        answer_id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        project_id: r.get("project_id"),
        question: r.get("question"),
        text: r.get("text"),
        assessed: r.get("assessed"),
    }))
}

/// Store the verdict: the answer's own-brand fields, one `answer_brands` row for each brand, and
/// `assessed = true`. The deterministic name match and the judge both count for `brand_mentioned`.
pub async fn store_verdict(
    pool: &PgPool,
    answer: &AnswerContext,
    own: &BrandSpec,
    competitors: &[BrandSpec],
    verdict: &Verdict,
) -> Result<()> {
    let mine = verdict.own(own);
    let mentioned = mine.is_some() || own.is_named_in(&answer.text);
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM answer_brands WHERE answer_id = $1")
        .bind(answer.answer_id)
        .execute(&mut *tx)
        .await?;
    for b in &verdict.brands {
        let owned = own.is_called(&b.name);
        let name = if owned {
            own.name.as_str()
        } else {
            competitors
                .iter()
                .find(|c| c.is_called(&b.name))
                .map_or(b.name.as_str(), |c| c.name.as_str())
        };
        sqlx::query(
            "INSERT INTO answer_brands (answer_id, name, owned, recommended, list_rank, sentiment)
             VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (answer_id, name) DO NOTHING",
        )
        .bind(answer.answer_id)
        .bind(name)
        .bind(owned)
        .bind(b.recommended)
        .bind(b.rank)
        .bind(&b.sentiment)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE answers SET assessed = true, brand_mentioned = $2, brand_recommended = $3, position = $4, sentiment = $5
         WHERE id = $1",
    )
    .bind(answer.answer_id)
    .bind(mentioned)
    .bind(mine.is_some_and(|b| b.recommended))
    .bind(mine.and_then(|b| b.rank))
    .bind(mine.and_then(|b| b.sentiment.clone()))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The judge mode now in force.
pub async fn judge_mode(pool: &PgPool) -> Result<crate::tracking_parse::JudgeMode> {
    let mode: String = sqlx::query_scalar("SELECT mode FROM judge_state")
        .fetch_one(pool)
        .await?;
    Ok(crate::tracking_parse::JudgeMode::parse(&mode))
}

/// Count one checked answer. When the sample is full, choose the mode. Returns the mode set, if any.
pub async fn record_agreement(
    pool: &PgPool,
    agreed: bool,
) -> Result<Option<crate::tracking_parse::JudgeMode>> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "UPDATE judge_state SET samples = samples + 1, agreed = agreed + $1 WHERE mode = 'unchecked'
         RETURNING samples, agreed",
    )
    .bind(i32::from(agreed))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let mode = crate::tracking_parse::decide_mode(row.get("samples"), row.get("agreed"));
    if let Some(mode) = mode {
        sqlx::query("UPDATE judge_state SET mode = $1, decided_at = now()")
            .bind(mode.as_str())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_scan_keeps_the_schedule_and_does_not_pile_up_missed_scans() {
        let t0: DateTime<Utc> = "2026-11-26T09:00:00Z".parse().unwrap();
        let late = t0 + Duration::minutes(3);
        assert_eq!(
            Cadence::Daily.next_after(t0, late),
            t0 + Duration::days(1),
            "a late tick keeps the planned time"
        );
        let long_down = t0 + Duration::days(10);
        assert_eq!(
            Cadence::Weekly.next_after(t0, long_down),
            long_down + Duration::days(7),
            "no catch-up burst"
        );
        assert_eq!(
            Cadence::EveryOtherDay.next_after(t0, t0),
            t0 + Duration::days(2)
        );
        assert_eq!(
            "every_other_day".parse::<Cadence>(),
            Ok(Cadence::EveryOtherDay)
        );
    }
}
