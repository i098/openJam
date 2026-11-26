//! SQL for the metrics. Postgres counts, Rust turns counts into rates (`metrics.rs`).
//!
//! Only answers the judge has assessed count, and only for prompts that are tracked and scored.
//! Every query reads two windows at once: the current one and the one before it.

use super::metrics::AnswerTally;
use chrono::{DateTime, Utc};
use openjam_core::Result;
use sqlx::PgPool;
use std::collections::BTreeMap;
use uuid::Uuid;

/// What to measure: one project, two windows, optional filters.
pub struct Scope {
    pub project_id: Uuid,
    pub prev_start: DateTime<Utc>,
    pub cur_start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub platform: Option<String>,
    pub intent: Option<String>,
    pub topic: Option<String>,
    /// Text the prompt contains. Escaped for `ILIKE` by `like_pattern`.
    pub query: Option<String>,
}

/// How to group answers.
#[derive(Clone, Copy)]
pub enum Split {
    All,
    Day,
    Platform,
    Prompt,
    Topic,
}

impl Split {
    fn sql(self) -> &'static str {
        match self {
            Self::All => "'all'",
            Self::Day => "to_char(a.observed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD')",
            Self::Platform => "a.platform",
            Self::Prompt => "a.prompt_id::text",
            Self::Topic => "COALESCE(p.topic, 'General')",
        }
    }
}

// Parameters $1 to $4 are the prompt filter, $5 to $8 the answer filter (see `bind`).
const PROMPT_FILTER: &str = "p.project_id = $1 AND p.list = 'tracked' AND p.scored
    AND ($2::text IS NULL OR p.intent = $2)
    AND ($3::text IS NULL OR p.topic = $3)
    AND ($4::text IS NULL OR p.text ILIKE '%' || $4 || '%')";

const ANSWER_FILTER: &str = "a.assessed AND a.observed_at >= $5 AND a.observed_at <= $6
    AND ($7::text IS NULL OR a.platform = $7)";

/// Escape `%`, `_` and `\` so a search text matches itself.
pub fn like_pattern(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Counts of one group in the current window and the window before.
#[derive(Clone, Debug, Default)]
pub struct Pair<T> {
    pub cur: T,
    pub prev: T,
}

/// Mentions of one competitor in a group.
#[derive(Clone, Copy, Debug, Default)]
pub struct BrandTally {
    pub mentions: u64,
    pub position_sum: i64,
    pub position_count: u64,
    /// Answers that cite the competitor's domain.
    pub cited: u64,
}

#[derive(Clone, Debug)]
pub struct Brand {
    pub name: String,
    pub domain: Option<String>,
    pub tally: Pair<BrandTally>,
}

type TallyRow = (bool, String, i64, i64, i64, i64, i64, i64);

/// Answer counts for each group, keyed by the group value.
pub async fn tallies(
    pool: &PgPool,
    scope: &Scope,
    split: Split,
) -> Result<BTreeMap<String, Pair<AnswerTally>>> {
    let sql = format!(
        "SELECT a.observed_at >= $8, {key}, count(*),
                count(*) FILTER (WHERE a.brand_mentioned),
                count(*) FILTER (WHERE a.brand_recommended),
                count(*) FILTER (WHERE a.own_domain_cited),
                COALESCE(sum(a.position), 0)::bigint, count(a.position)
         FROM answers a JOIN prompts p ON p.id = a.prompt_id
         WHERE {PROMPT_FILTER} AND {ANSWER_FILTER}
         GROUP BY 1, 2",
        key = split.sql()
    );
    let rows: Vec<TallyRow> = bind(sqlx::query_as(&sql), scope).fetch_all(pool).await?;
    let mut out: BTreeMap<String, Pair<AnswerTally>> = BTreeMap::new();
    for (current, key, answers, mentioned, recommended, cited, position_sum, position_count) in rows
    {
        let pair = out.entry(key).or_default();
        let side = if current {
            &mut pair.cur
        } else {
            &mut pair.prev
        };
        *side = AnswerTally {
            answers: answers as u64,
            mentioned: mentioned as u64,
            recommended: recommended as u64,
            cited: cited as u64,
            position_sum,
            position_count: position_count as u64,
        };
    }
    Ok(out)
}

type BrandRow = (bool, String, String, Option<String>, i64, i64, i64, i64);

/// Competitor mentions for each group, keyed by the group value.
pub async fn competitors(
    pool: &PgPool,
    scope: &Scope,
    split: Split,
) -> Result<BTreeMap<String, Vec<Brand>>> {
    let sql = format!(
        "SELECT a.observed_at >= $8, {key}, b.name, c.domain, count(*),
                COALESCE(sum(b.list_rank), 0)::bigint, count(b.list_rank),
                count(*) FILTER (WHERE c.domain IS NOT NULL AND EXISTS (
                    SELECT 1 FROM answer_citations ac
                    WHERE ac.answer_id = a.id AND lower(ac.domain) = lower(c.domain)))
         FROM answers a
         JOIN prompts p ON p.id = a.prompt_id
         JOIN answer_brands b ON b.answer_id = a.id AND NOT b.owned
         LEFT JOIN competitors c ON c.project_id = p.project_id AND c.name = b.name
         WHERE {PROMPT_FILTER} AND {ANSWER_FILTER}
         GROUP BY 1, 2, 3, 4",
        key = split.sql()
    );
    let rows: Vec<BrandRow> = bind(sqlx::query_as(&sql), scope).fetch_all(pool).await?;
    let mut out: BTreeMap<String, Vec<Brand>> = BTreeMap::new();
    for (current, key, name, domain, mentions, position_sum, position_count, cited) in rows {
        let brands = out.entry(key).or_default();
        let index = match brands.iter().position(|b| b.name == name) {
            Some(index) => index,
            None => {
                brands.push(Brand {
                    name,
                    domain,
                    tally: Pair::default(),
                });
                brands.len() - 1
            }
        };
        let tally = BrandTally {
            mentions: mentions as u64,
            position_sum,
            position_count: position_count as u64,
            cited: cited as u64,
        };
        let pair = &mut brands[index].tally;
        if current {
            pair.cur = tally;
        } else {
            pair.prev = tally;
        }
    }
    Ok(out)
}

/// A tracked prompt that passes the filters.
#[derive(sqlx::FromRow)]
pub struct PromptInfo {
    pub id: Uuid,
    pub text: String,
    pub topic: Option<String>,
    pub persona: Option<String>,
    pub intent: Option<String>,
}

/// Tracked, scored prompts that pass the prompt filters, oldest first.
pub async fn prompts(pool: &PgPool, scope: &Scope) -> Result<Vec<PromptInfo>> {
    let sql = format!(
        "SELECT p.id, p.text, p.topic, p.persona, p.intent FROM prompts p
         WHERE {PROMPT_FILTER} ORDER BY p.created_at, p.id"
    );
    Ok(bind_prompt(sqlx::query_as(&sql), scope)
        .fetch_all(pool)
        .await?)
}

/// Tracked, scored prompts for each topic.
pub async fn prompt_counts(pool: &PgPool, project_id: Uuid) -> Result<BTreeMap<String, u64>> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT COALESCE(topic, 'General'), count(*) FROM prompts
         WHERE project_id = $1 AND list = 'tracked' AND scored GROUP BY 1",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(t, n)| (t, n as u64)).collect())
}

/// A cited domain in the current window.
pub struct Source {
    pub domain: String,
    pub citations: u64,
    pub answers: u64,
}

/// The domains answers cite in the current window, most cited first.
pub async fn sources(pool: &PgPool, scope: &Scope) -> Result<Vec<Source>> {
    let sql = format!(
        "SELECT lower(ac.domain), count(*), count(DISTINCT a.id)
         FROM answers a
         JOIN prompts p ON p.id = a.prompt_id
         JOIN answer_citations ac ON ac.answer_id = a.id
         WHERE {PROMPT_FILTER} AND {ANSWER_FILTER} AND a.observed_at >= $8
         GROUP BY 1 ORDER BY 2 DESC, 1"
    );
    let rows: Vec<(String, i64, i64)> = bind(sqlx::query_as(&sql), scope).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|(domain, citations, answers)| Source {
            domain,
            citations: citations as u64,
            answers: answers as u64,
        })
        .collect())
}

fn bind<'q, O>(
    query: sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>,
    scope: &'q Scope,
) -> sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments> {
    bind_prompt(query, scope)
        .bind(scope.prev_start)
        .bind(scope.end)
        .bind(scope.platform.as_deref())
        .bind(scope.cur_start)
}

fn bind_prompt<'q, O>(
    query: sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>,
    scope: &'q Scope,
) -> sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments> {
    query
        .bind(scope.project_id)
        .bind(scope.intent.as_deref())
        .bind(scope.topic.as_deref())
        .bind(scope.query.as_deref().map(like_pattern))
}
