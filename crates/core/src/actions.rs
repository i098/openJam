//! Actions: the status machine, the history, checks, and the technical findings.
//!
//! Status is never set by hand. It comes from the event history (`derive_status`), and every
//! transition goes through `record`, which checks the move and stores the event and the status
//! in one transaction. Fetching pages is not here: the worker fetches and passes in a `Probe`.

use crate::{Error, Result, auth::sha256_hex, jobs, tracking_parse::normalize};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::{collections::BTreeSet, str::FromStr};
use uuid::Uuid;

pub const QUEUE_CHECK: &str = "check";

pub const KINDS: [&str; 5] = [
    "page_edit",
    "new_page",
    "outreach",
    "technical",
    "community",
];
pub const PROBLEMS: [&str; 10] = [
    "list_absent",
    "list_low_rank",
    "wrong_claim",
    "page_read_rival_won",
    "missing_page",
    "technical_defect",
    "engine_gap",
    "community_absent",
    "unclear_positioning",
    "other",
];

// ---- status ---------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Suggested,
    InProgress,
    Sent,
    Done,
    Verified,
    Unverified,
    Dismissed,
    Superseded,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Suggested => "suggested",
            Self::InProgress => "in_progress",
            Self::Sent => "sent",
            Self::Done => "done",
            Self::Verified => "verified",
            Self::Unverified => "unverified",
            Self::Dismissed => "dismissed",
            Self::Superseded => "superseded",
        }
    }
}

impl FromStr for Status {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        Ok(match s {
            "suggested" => Self::Suggested,
            "in_progress" => Self::InProgress,
            "sent" => Self::Sent,
            "done" => Self::Done,
            "verified" => Self::Verified,
            "unverified" => Self::Unverified,
            "dismissed" => Self::Dismissed,
            "superseded" => Self::Superseded,
            other => return Err(format!("unknown status {other}")),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Suggested,
    Started,
    Sent,
    MarkedDone,
    Delivered,
    VerificationPassed,
    VerificationFailed,
    Dismissed,
    Superseded,
    Reopened,
    Restored,
}

impl Event {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Suggested => "suggested",
            Self::Started => "started",
            Self::Sent => "sent",
            Self::MarkedDone => "marked_done",
            Self::Delivered => "delivered",
            Self::VerificationPassed => "verification_passed",
            Self::VerificationFailed => "verification_failed",
            Self::Dismissed => "dismissed",
            Self::Superseded => "superseded",
            Self::Reopened => "reopened",
            Self::Restored => "restored",
        }
    }
}

impl FromStr for Event {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        Ok(match s {
            "suggested" => Self::Suggested,
            "started" => Self::Started,
            "sent" => Self::Sent,
            "marked_done" => Self::MarkedDone,
            "delivered" => Self::Delivered,
            "verification_passed" => Self::VerificationPassed,
            "verification_failed" => Self::VerificationFailed,
            "dismissed" => Self::Dismissed,
            "superseded" => Self::Superseded,
            "reopened" => Self::Reopened,
            "restored" => Self::Restored,
            other => return Err(format!("unknown event {other}")),
        })
    }
}

/// The status after `events`, oldest first. A restore returns to the status before the dismissal.
pub fn derive_status(events: &[Event]) -> Status {
    let mut status = Status::Suggested;
    let mut before_dismissal = Status::Suggested;
    let checked = |s: Status| matches!(s, Status::Done | Status::Verified | Status::Unverified);
    for event in events {
        status = match event {
            Event::Suggested | Event::Reopened => Status::Suggested,
            Event::Started if matches!(status, Status::Suggested | Status::Unverified) => {
                Status::InProgress
            }
            Event::Sent => Status::Sent,
            Event::MarkedDone | Event::Delivered => Status::Done,
            Event::VerificationPassed if checked(status) => Status::Verified,
            Event::VerificationFailed if checked(status) => Status::Unverified,
            Event::Dismissed => {
                before_dismissal = status;
                Status::Dismissed
            }
            Event::Restored if status == Status::Dismissed => before_dismissal,
            Event::Superseded => Status::Superseded,
            _ => status,
        };
    }
    status
}

/// May `event` happen now? `Ok(true)`: record it. `Ok(false)`: nothing to do, the action is
/// already there. `Err`: the move is not allowed, with the reason for the caller.
pub fn allow(kind: &str, status: Status, event: Event) -> std::result::Result<bool, String> {
    use Status::*;
    match event {
        Event::Started => match status {
            InProgress => Ok(false),
            Suggested | Unverified => Ok(true),
            _ => Err(format!("a {} action cannot be started", status.as_str())),
        },
        Event::Sent if kind != "outreach" => Err("only an outreach action can be sent".into()),
        Event::Sent => match status {
            Sent => Ok(false),
            Suggested | InProgress => Ok(true),
            _ => Err(format!("a {} action cannot be sent", status.as_str())),
        },
        Event::MarkedDone | Event::Delivered => match status {
            Suggested | InProgress | Sent | Unverified => Ok(true),
            Done | Verified => Ok(false),
            _ => Err(format!("a {} action cannot be completed", status.as_str())),
        },
        Event::Dismissed => match status {
            Dismissed => Ok(false),
            Superseded => Err("a superseded action cannot be dismissed".into()),
            _ => Ok(true),
        },
        Event::Restored => match status {
            Dismissed => Ok(true),
            _ => Err("only a dismissed action can be restored".into()),
        },
        Event::VerificationPassed | Event::VerificationFailed => match status {
            Done | Verified | Unverified => Ok(true),
            _ => Err("only a done action can be checked".into()),
        },
        Event::Suggested | Event::Superseded | Event::Reopened => Ok(true),
    }
}

pub struct NewEvent<'a> {
    pub event: Event,
    pub actor: &'a str,
    pub url: Option<&'a str>,
    pub evidence: Option<&'a str>,
    pub note: Option<&'a str>,
}

impl<'a> NewEvent<'a> {
    pub fn new(event: Event, actor: &'a str) -> Self {
        Self {
            event,
            actor,
            url: None,
            evidence: None,
            note: None,
        }
    }
}

/// Apply one event to an action in the workspace. Returns the new status. A move that is not
/// allowed is a `Conflict`.
pub async fn record(
    pool: &PgPool,
    workspace_id: Uuid,
    action_id: Uuid,
    ev: &NewEvent<'_>,
) -> Result<Status> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT a.kind, a.status FROM actions a JOIN projects p ON p.id = a.project_id
         WHERE a.id = $1 AND p.workspace_id = $2 FOR UPDATE OF a",
    )
    .bind(action_id)
    .bind(workspace_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Error::NotFound("action"))?;
    let kind: String = row.get("kind");
    let current = row
        .get::<String, _>("status")
        .parse::<Status>()
        .map_err(Error::Internal)?;
    if !allow(&kind, current, ev.event).map_err(Error::Conflict)? {
        return Ok(current);
    }
    let status = insert_event(&mut tx, action_id, ev).await?;
    tx.commit().await?;
    Ok(status)
}

/// Insert the event, derive the status from the whole history and store it. The caller holds
/// the row lock.
pub async fn insert_event(
    tx: &mut sqlx::PgConnection,
    action_id: Uuid,
    ev: &NewEvent<'_>,
) -> Result<Status> {
    sqlx::query(
        "INSERT INTO action_events (action_id, type, actor, url, evidence, note)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(action_id)
    .bind(ev.event.as_str())
    .bind(ev.actor)
    .bind(ev.url)
    .bind(ev.evidence)
    .bind(ev.note)
    .execute(&mut *tx)
    .await?;
    let types: Vec<String> =
        sqlx::query_scalar("SELECT type FROM action_events WHERE action_id = $1 ORDER BY id")
            .bind(action_id)
            .fetch_all(&mut *tx)
            .await?;
    let events: Vec<Event> = types.iter().filter_map(|t| t.parse().ok()).collect();
    let status = derive_status(&events);
    sqlx::query("UPDATE actions SET status = $2 WHERE id = $1")
        .bind(action_id)
        .bind(status.as_str())
        .execute(&mut *tx)
        .await?;
    Ok(status)
}

/// Report an action done, by the user or by an agent. With a check spec and a place to look, the
/// worker checks it once; until the check passes the action stays `done`.
pub async fn complete(
    pool: &PgPool,
    workspace_id: Uuid,
    action_id: Uuid,
    actor: &str,
    url: Option<&str>,
    note: Option<&str>,
) -> Result<Status> {
    let ev = NewEvent {
        url,
        note,
        ..NewEvent::new(Event::MarkedDone, actor)
    };
    let status = record(pool, workspace_id, action_id, &ev).await?;
    queue_check(pool, action_id, url).await?;
    Ok(status)
}

/// Report done with the secret from the action's prompt. The token is the only credential.
pub async fn complete_with_token(
    pool: &PgPool,
    token: &str,
    url: Option<&str>,
    note: Option<&str>,
) -> Result<Status> {
    let row = sqlx::query(
        "SELECT a.id, p.workspace_id FROM actions a JOIN projects p ON p.id = a.project_id
         WHERE a.completion_token = $1",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?
    .ok_or(Error::NotFound("action"))?;
    complete(
        pool,
        row.get("workspace_id"),
        row.get("id"),
        "agent",
        url,
        note,
    )
    .await
}

/// Ask for a check of a done action. `url` is where the finished work lives; without it the
/// check uses the action's target page, or none for a site-wide check.
pub async fn queue_check(pool: &PgPool, action_id: Uuid, url: Option<&str>) -> Result<()> {
    let spec: Option<Value> = sqlx::query_scalar("SELECT check_spec FROM actions WHERE id = $1")
        .bind(action_id)
        .fetch_one(pool)
        .await?;
    if spec.is_some() {
        jobs::enqueue(
            pool,
            QUEUE_CHECK,
            &json!({"action_id": action_id, "url": url}),
            None,
            None,
        )
        .await?;
    }
    Ok(())
}

// ---- checks ---------------------------------------------------------------------------------

/// What the check looks for. Stored with the action when research proposes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CheckSpec {
    /// A new page answers with a page that names the brand.
    PageLive,
    /// An edited page differs from the page research read (`baseline` is its hash).
    PageChanged {
        baseline: Option<String>,
    },
    RobotsAllowsAi,
    Sitemap,
    LlmsTxt,
    Indexable,
}

impl CheckSpec {
    /// A check about the whole site needs no page URL.
    pub fn is_site_wide(&self) -> bool {
        !matches!(self, Self::PageLive | Self::PageChanged { .. })
    }
}

/// One fetched URL. `status: None`: the fetch failed.
#[derive(Clone, Debug, Default)]
pub struct Fetched {
    pub url: String,
    pub status: Option<u16>,
    pub text: String,
}

impl Fetched {
    pub fn ok(&self) -> bool {
        self.status.is_some_and(|s| (200..300).contains(&s))
    }
}

/// The pages a check or a technical review looks at. Absent: not fetched.
#[derive(Clone, Debug, Default)]
pub struct Probe {
    pub page: Option<Fetched>,
    pub home: Option<Fetched>,
    pub robots: Option<Fetched>,
    pub sitemap: Option<Fetched>,
    pub llms: Option<Fetched>,
}

pub fn content_hash(text: &str) -> String {
    sha256_hex(&normalize(text))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CheckOutcome {
    pub passed: bool,
    pub evidence: String,
}

fn outcome(passed: bool, evidence: impl Into<String>) -> CheckOutcome {
    CheckOutcome {
        passed,
        evidence: evidence.into(),
    }
}

pub fn evaluate(spec: &CheckSpec, brand: &str, probe: &Probe) -> CheckOutcome {
    let absent = || outcome(false, "the page was not fetched");
    match spec {
        CheckSpec::PageLive => {
            let Some(page) = probe.page.as_ref() else {
                return absent();
            };
            if !page.ok() {
                return outcome(false, format!("{} did not answer with a page", page.url));
            }
            if normalize(&page.text).contains(normalize(brand).trim_end()) {
                outcome(true, format!("{} is live and names {brand}", page.url))
            } else {
                outcome(
                    false,
                    format!("{} is live but does not name {brand}", page.url),
                )
            }
        }
        CheckSpec::PageChanged { baseline } => {
            let Some(page) = probe.page.as_ref() else {
                return absent();
            };
            if !page.ok() {
                return outcome(false, format!("{} did not answer with a page", page.url));
            }
            match baseline {
                Some(old) if *old == content_hash(&page.text) => outcome(
                    false,
                    format!("{} has the same text as when research read it", page.url),
                ),
                _ => outcome(true, format!("{} changed since research read it", page.url)),
            }
        }
        CheckSpec::RobotsAllowsAi => match probe.robots.as_ref() {
            Some(r) if r.ok() => {
                let blocked = robots_blocked_bots(&r.text);
                if blocked.is_empty() {
                    outcome(true, "robots.txt lets the AI crawlers in")
                } else {
                    outcome(
                        false,
                        format!("robots.txt still blocks {}", blocked.join(", ")),
                    )
                }
            }
            // No robots.txt means no block.
            Some(r) if r.status == Some(404) => {
                outcome(true, "there is no robots.txt to block them")
            }
            _ => outcome(false, "robots.txt could not be read"),
        },
        CheckSpec::Sitemap => match probe.sitemap.as_ref() {
            Some(s) if s.ok() => outcome(true, format!("{} answers", s.url)),
            _ => outcome(false, "sitemap.xml does not answer"),
        },
        CheckSpec::LlmsTxt => match probe.llms.as_ref() {
            Some(s) if s.ok() && !s.text.trim().is_empty() => {
                outcome(true, format!("{} answers", s.url))
            }
            _ => outcome(false, "llms.txt does not answer"),
        },
        CheckSpec::Indexable => match probe.home.as_ref() {
            Some(h) if h.ok() && !has_noindex(&h.text) => {
                outcome(true, "the home page can be indexed")
            }
            Some(h) if h.ok() => outcome(false, "the home page still says noindex"),
            _ => outcome(false, "the home page could not be read"),
        },
    }
}

/// The AI crawlers a robots.txt shuts out of the whole site.
pub const AI_BOTS: [&str; 6] = [
    "gptbot",
    "oai-searchbot",
    "chatgpt-user",
    "claudebot",
    "perplexitybot",
    "google-extended",
];

/// The AI crawlers `robots` shuts out with `Disallow: /`. A named group wins over `*`.
pub fn robots_blocked_bots(robots: &str) -> Vec<String> {
    // Groups: (agents, disallows-everything)
    let mut groups: Vec<(Vec<String>, bool)> = Vec::new();
    let mut agents: Vec<String> = Vec::new();
    let mut in_rules = false;
    for line in robots.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        let (field, value) = (field.trim().to_ascii_lowercase(), value.trim());
        match field.as_str() {
            "user-agent" => {
                if in_rules {
                    agents.clear();
                    in_rules = false;
                }
                agents.push(value.to_ascii_lowercase());
                groups.push((agents.clone(), false));
            }
            "disallow" | "allow" => {
                in_rules = true;
                if field == "disallow" && value == "/" {
                    for g in groups.iter_mut().filter(|g| g.0 == agents) {
                        g.1 = true;
                    }
                }
                if field == "allow" && value == "/" {
                    for g in groups.iter_mut().filter(|g| g.0 == agents) {
                        g.1 = false;
                    }
                }
            }
            _ => {}
        }
    }
    let blocks = |agent: &str| {
        let named = groups.iter().rev().find(|g| g.0.iter().any(|a| a == agent));
        match named {
            Some(g) => g.1,
            None => groups
                .iter()
                .rev()
                .find(|g| g.0.iter().any(|a| a == "*"))
                .is_some_and(|g| g.1),
        }
    };
    AI_BOTS
        .iter()
        .filter(|b| blocks(b))
        .map(|b| (*b).to_owned())
        .collect()
}

/// True when the page's robots meta tag says `noindex`.
pub fn has_noindex(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.match_indices("<meta").any(|(i, _)| {
        let tag = &lower[i..lower[i..].find('>').map_or(lower.len(), |e| i + e)];
        tag.contains("robots") && tag.contains("noindex")
    })
}

pub struct CheckContext {
    pub action_id: Uuid,
    pub project_id: Uuid,
    pub brand: String,
    pub site_url: String,
    pub target_url: Option<String>,
    pub spec: CheckSpec,
    pub status: Status,
}

pub async fn load_check(pool: &PgPool, action_id: Uuid) -> Result<Option<CheckContext>> {
    let Some(row) = sqlx::query(
        "SELECT a.id, a.project_id, a.target_url, a.check_spec, a.status, p.name, p.site_url
         FROM actions a JOIN projects p ON p.id = a.project_id WHERE a.id = $1",
    )
    .bind(action_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let Some(spec) = row
        .get::<Option<Value>, _>("check_spec")
        .and_then(|v| serde_json::from_value(v).ok())
    else {
        return Ok(None);
    };
    Ok(Some(CheckContext {
        action_id: row.get("id"),
        project_id: row.get("project_id"),
        brand: row.get("name"),
        site_url: row.get("site_url"),
        target_url: row.get("target_url"),
        spec,
        status: row
            .get::<String, _>("status")
            .parse()
            .map_err(Error::Internal)?,
    }))
}

/// Store a check result. It counts only while the action is done, verified or unverified.
pub async fn store_check(
    pool: &PgPool,
    ctx: &CheckContext,
    url: Option<&str>,
    result: &CheckOutcome,
    at: DateTime<Utc>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let status: String = sqlx::query_scalar("SELECT status FROM actions WHERE id = $1 FOR UPDATE")
        .bind(ctx.action_id)
        .fetch_one(&mut *tx)
        .await?;
    if !matches!(status.as_str(), "done" | "verified" | "unverified") {
        return Ok(());
    }
    sqlx::query("UPDATE actions SET check_result = $2 WHERE id = $1")
        .bind(ctx.action_id)
        .bind(json!({"passed": result.passed, "evidence": result.evidence, "url": url, "at": at}))
        .execute(&mut *tx)
        .await?;
    let event = if result.passed {
        Event::VerificationPassed
    } else {
        Event::VerificationFailed
    };
    let ev = NewEvent {
        url,
        evidence: Some(&result.evidence),
        ..NewEvent::new(event, "jam")
    };
    insert_event(&mut tx, ctx.action_id, &ev).await?;
    tx.commit().await?;
    Ok(())
}

// ---- proposals ------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Impact {
    Low,
    Medium,
    High,
}

impl Impact {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// High 3, medium 2, low 1.
    pub fn weight(self) -> u32 {
        match self {
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
        }
    }
}

impl FromStr for Impact {
    type Err = ();
    fn from_str(s: &str) -> std::result::Result<Self, ()> {
        match s.trim().to_ascii_lowercase().as_str() {
            "high" => Ok(Self::High),
            "medium" => Ok(Self::Medium),
            "low" => Ok(Self::Low),
            _ => Err(()),
        }
    }
}

/// One piece of work an agent proposes. Two proposals of the same work merge into one.
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    /// `new_page`, `page_edit` or `technical`.
    pub kind: &'static str,
    pub problem: &'static str,
    pub title: String,
    pub why: String,
    pub impact: Impact,
    pub target_url: Option<String>,
    /// Tracked prompts this work would help.
    pub prompt_ids: Vec<Uuid>,
    pub sources: Vec<String>,
    pub details: Vec<String>,
    pub topic: Option<String>,
    pub persona: Option<String>,
    pub intent: Option<String>,
    pub change_kind: Option<String>,
    /// The article plan for a new page: `{angle, outline: [{heading, intent}]}`.
    pub plan: Option<Value>,
    pub check: Option<CheckSpec>,
}

/// Lower case URL without scheme, `www.`, fragment or trailing slash.
pub fn url_key(url: &str) -> String {
    let u = url
        .trim()
        .split('#')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let u = u.split_once("://").map_or(u.as_str(), |(_, r)| r);
    let u = u.strip_prefix("www.").unwrap_or(u);
    u.trim_end_matches('/').to_owned()
}

impl Proposal {
    /// Stable identity of the work, so a later run finds the same action.
    pub fn fingerprint(&self) -> String {
        let key = match (&self.target_url, &self.check) {
            (Some(url), _) => url_key(url),
            (None, Some(spec)) if spec.is_site_wide() => {
                serde_json::to_string(spec).unwrap_or_default()
            }
            _ => normalize(&self.title).trim().to_owned(),
        };
        format!("{}:{key}", self.kind)
    }

    /// Impact weight times the number of tracked prompts it helps (at least one).
    pub fn score(&self) -> f64 {
        f64::from(self.impact.weight()) * self.prompt_ids.len().max(1) as f64
    }
}

fn union<T: Clone + Ord>(into: &mut Vec<T>, from: &[T]) {
    for item in from {
        if !into.contains(item) {
            into.push(item.clone());
        }
    }
}

/// Merge proposals of the same work, then order them best first: score, then title.
pub fn merge_and_rank(proposals: Vec<Proposal>) -> Vec<Proposal> {
    let mut merged: Vec<Proposal> = Vec::new();
    let mut seen = BTreeSet::new();
    for p in proposals {
        let key = p.fingerprint();
        if seen.insert(key.clone()) {
            merged.push(p);
            continue;
        }
        let Some(into) = merged.iter_mut().find(|m| m.fingerprint() == key) else {
            continue;
        };
        union(&mut into.prompt_ids, &p.prompt_ids);
        union(&mut into.sources, &p.sources);
        union(&mut into.details, &p.details);
        if p.impact > into.impact {
            into.impact = p.impact;
            into.title = p.title;
        }
        into.target_url = into.target_url.take().or(p.target_url);
        into.plan = into.plan.take().or(p.plan);
    }
    merged.sort_by(|a, b| {
        b.score()
            .total_cmp(&a.score())
            .then_with(|| a.title.cmp(&b.title))
    });
    merged
}

/// Findings of the technical agent, from fetched site files. No model is involved.
pub fn technical_findings(site_url: &str, probe: &Probe) -> Vec<Proposal> {
    let site = site_url.trim_end_matches('/');
    let finding = |title: &str, why: &str, impact, check: CheckSpec, file: &str| Proposal {
        kind: "technical",
        problem: "technical_defect",
        title: title.to_owned(),
        why: why.to_owned(),
        impact,
        target_url: (!file.is_empty()).then(|| format!("{site}/{file}")),
        prompt_ids: vec![],
        sources: vec![],
        details: vec![],
        topic: None,
        persona: None,
        intent: None,
        change_kind: None,
        plan: None,
        check: Some(check),
    };
    let mut out = Vec::new();
    if let Some(robots) = probe.robots.as_ref().filter(|r| r.ok()) {
        let blocked = robots_blocked_bots(&robots.text);
        if !blocked.is_empty() {
            let mut p = finding(
                "Let AI crawlers read your site",
                "robots.txt shuts AI crawlers out, so AI search cannot read or cite your pages.",
                Impact::High,
                CheckSpec::RobotsAllowsAi,
                "robots.txt",
            );
            p.details = vec![format!("robots.txt blocks: {}", blocked.join(", "))];
            out.push(p);
        }
    }
    let sitemap_listed = probe
        .robots
        .as_ref()
        .is_some_and(|r| r.ok() && r.text.to_ascii_lowercase().contains("sitemap:"));
    if probe.sitemap.as_ref().is_some_and(|s| !s.ok()) && !sitemap_listed {
        out.push(finding(
            "Publish a sitemap",
            "A sitemap lists your pages for crawlers. Yours does not answer.",
            Impact::Medium,
            CheckSpec::Sitemap,
            "sitemap.xml",
        ));
    }
    if probe.llms.as_ref().is_some_and(|s| !s.ok()) {
        out.push(finding(
            "Add an llms.txt file",
            "llms.txt gives AI assistants a short guide to your site. Yours is missing.",
            Impact::Low,
            CheckSpec::LlmsTxt,
            "llms.txt",
        ));
    }
    if probe
        .home
        .as_ref()
        .is_some_and(|h| h.ok() && has_noindex(&h.text))
    {
        out.push(finding(
            "Remove noindex from the home page",
            "The home page tells search engines not to index it.",
            Impact::High,
            CheckSpec::Indexable,
            "",
        ));
    }
    out
}

/// The text an agent follows to do the action and report it through the API.
pub fn agent_prompt(
    kind: &str,
    title: &str,
    why: &str,
    target_url: Option<&str>,
    details: &[String],
    token: &str,
) -> String {
    let mut out = format!("Task: {title}\nWhy: {why}\n");
    if let Some(url) = target_url {
        out.push_str(&format!("Page: {url}\n"));
    }
    for d in details {
        out.push_str(&format!("- {d}\n"));
    }
    let report = if kind == "outreach" {
        "When the pitch is sent, call POST /v1/actions/{id}/sent. When the work is done,"
    } else {
        "When the work is done,"
    };
    out.push_str(&format!(
        "{report} call POST /v1/action-completions/{token} with {{\"url\": \"<where the work lives>\"}}."
    ));
    out
}

/// A new secret for `actions.completion_token`.
pub fn new_completion_token() -> String {
    format!("gac_{}", hex::encode(rand::random::<[u8; 16]>()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fetched(status: u16, text: &str) -> Fetched {
        Fetched {
            url: "https://acme.test/x".into(),
            status: Some(status),
            text: text.into(),
        }
    }

    #[test]
    fn status_follows_the_history_and_a_restore_goes_back() {
        use Event::*;
        let status = |e: &[Event]| derive_status(e).as_str();
        assert_eq!(status(&[Suggested]), "suggested");
        assert_eq!(status(&[Suggested, Started]), "in_progress");
        assert_eq!(status(&[Suggested, Started, MarkedDone]), "done");
        assert_eq!(
            status(&[Suggested, MarkedDone, VerificationPassed]),
            "verified"
        );
        assert_eq!(
            status(&[Suggested, MarkedDone, VerificationFailed, Started]),
            "in_progress",
            "an unverified action can be started again"
        );
        assert_eq!(status(&[Suggested, Started, Dismissed]), "dismissed");
        assert_eq!(
            status(&[Suggested, Started, Dismissed, Restored]),
            "in_progress",
            "restore returns to the status before the dismissal"
        );
        assert_eq!(
            status(&[Suggested, VerificationPassed]),
            "suggested",
            "a check of work nobody did changes nothing"
        );
        assert_eq!(status(&[Suggested, Superseded, Reopened]), "suggested");
    }

    #[test]
    fn moves_are_checked_against_status_and_kind() {
        use Status::*;
        assert_eq!(allow("outreach", Suggested, Event::Sent), Ok(true));
        assert!(allow("new_page", Suggested, Event::Sent).is_err());
        assert_eq!(allow("new_page", InProgress, Event::Started), Ok(false));
        assert_eq!(allow("new_page", Done, Event::MarkedDone), Ok(false));
        assert!(allow("new_page", Dismissed, Event::MarkedDone).is_err());
        assert!(allow("new_page", Suggested, Event::Restored).is_err());
        assert!(allow("new_page", Suggested, Event::VerificationPassed).is_err());
        assert!(allow("new_page", Superseded, Event::Dismissed).is_err());
    }

    #[test]
    fn robots_blocks_follow_the_most_specific_group() {
        let robots = "User-agent: *\nDisallow: /private\n\nUser-agent: GPTBot\nUser-agent: ClaudeBot\nDisallow: /\n";
        assert_eq!(robots_blocked_bots(robots), ["gptbot", "claudebot"]);
        let all = "User-agent: *\nDisallow: /\n\nUser-agent: PerplexityBot\nAllow: /\n";
        assert!(!robots_blocked_bots(all).contains(&"perplexitybot".to_owned()));
        assert!(robots_blocked_bots(all).contains(&"gptbot".to_owned()));
        assert!(robots_blocked_bots("User-agent: *\nDisallow:\n").is_empty());
        assert!(robots_blocked_bots("").is_empty());
    }

    #[test]
    fn noindex_is_read_from_the_robots_meta_tag_only() {
        assert!(has_noindex(
            r#"<head><meta name="robots" content="noindex, follow">"#
        ));
        assert!(!has_noindex(
            r#"<meta name="description" content="we never noindex">"#
        ));
        assert!(!has_noindex("<p>noindex</p>"));
    }

    #[test]
    fn technical_findings_come_from_the_fetched_files() {
        let probe = Probe {
            robots: Some(fetched(200, "User-agent: GPTBot\nDisallow: /")),
            sitemap: Some(fetched(404, "")),
            llms: Some(fetched(200, "# Acme")),
            home: Some(fetched(200, "<meta name=\"robots\" content=\"noindex\">")),
            ..Default::default()
        };
        let titles: Vec<_> = technical_findings("https://acme.test/", &probe)
            .into_iter()
            .map(|p| p.title)
            .collect();
        assert_eq!(
            titles,
            [
                "Let AI crawlers read your site",
                "Publish a sitemap",
                "Remove noindex from the home page"
            ]
        );
        let ok = Probe {
            robots: Some(fetched(200, "Sitemap: https://acme.test/s.xml")),
            sitemap: Some(fetched(404, "")),
            llms: Some(fetched(200, "x")),
            home: Some(fetched(200, "<p>hi</p>")),
            ..Default::default()
        };
        assert!(
            technical_findings("https://acme.test", &ok).is_empty(),
            "a sitemap named in robots.txt is enough"
        );
    }

    #[test]
    fn checks_pass_only_on_evidence() {
        let brand = "Acme CRM";
        let live = Probe {
            page: Some(fetched(200, "Why Acme CRM wins")),
            ..Default::default()
        };
        assert!(evaluate(&CheckSpec::PageLive, brand, &live).passed);
        let other = Probe {
            page: Some(fetched(200, "Nothing here")),
            ..Default::default()
        };
        assert!(!evaluate(&CheckSpec::PageLive, brand, &other).passed);
        assert!(!evaluate(&CheckSpec::PageLive, brand, &Probe::default()).passed);

        let same = CheckSpec::PageChanged {
            baseline: Some(content_hash("Why  Acme CRM wins")),
        };
        assert!(
            !evaluate(&same, brand, &live).passed,
            "white space does not count as a change"
        );
        assert!(evaluate(&same, brand, &other).passed);
        let down = Probe {
            page: Some(fetched(500, "oops")),
            ..Default::default()
        };
        assert!(!evaluate(&same, brand, &down).passed);
    }

    #[test]
    fn merged_proposals_rank_by_impact_times_prompts() {
        let p = |title: &str, impact, prompts: &[u128], url: Option<&str>| Proposal {
            kind: "page_edit",
            problem: "list_low_rank",
            title: title.into(),
            why: "w".into(),
            impact,
            target_url: url.map(Into::into),
            prompt_ids: prompts.iter().map(|n| Uuid::from_u128(*n)).collect(),
            sources: vec![],
            details: vec![],
            topic: None,
            persona: None,
            intent: None,
            change_kind: None,
            plan: None,
            check: None,
        };
        let ranked = merge_and_rank(vec![
            p(
                "Add pricing table",
                Impact::Low,
                &[1, 2, 3],
                Some("https://www.acme.test/pricing/"),
            ),
            p(
                "Rewrite intro",
                Impact::High,
                &[4],
                Some("https://acme.test/"),
            ),
            p(
                "Pricing table, better",
                Impact::Medium,
                &[3, 5],
                Some("https://acme.test/pricing#top"),
            ),
        ]);
        assert_eq!(ranked.len(), 2, "same page, same work: one action");
        assert_eq!(
            ranked[0].title, "Pricing table, better",
            "the stronger impact names it"
        );
        assert_eq!(ranked[0].prompt_ids.len(), 4, "prompts are unioned");
        assert_eq!(ranked[0].score(), 8.0);
        assert_eq!(ranked[1].score(), 3.0);
    }
}
