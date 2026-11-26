//! Research runs and action checks.
//!
//! A run reads the assessed scan answers, reads the pages they cited, and lets three agents
//! propose work: new pages and page edits (model calls), and technical fixes (site files, no
//! model). Their proposals are merged, ranked and stored as opportunities and actions.

use crate::{Outcome, Services, payload_id, text};
use chrono::{Duration, Utc};
use openjam_core::{
    actions::{
        self, CheckSpec, Fetched, Probe, Proposal, content_hash, evaluate, technical_findings,
        url_key,
    },
    jobs::Job,
    research::{self, Agent, Evidence},
    usage::{self, UsageEntry},
};
use openjam_engines::{EngineError, router::ModelTier};
use std::collections::{HashMap, HashSet};

const PAGES_PER_AGENT: usize = 6;
const MODEL_TRIES: usize = 2;
const WAIT_FOR_JUDGE: Duration = Duration::seconds(30);

/// Why a run stops without a result.
enum Stop {
    /// Try the job again later. A lost connection, a busy provider.
    Retry(String),
    /// The run cannot finish and a retry would not help. The message is shown to the user.
    Fail(String),
    /// Run again at this time (a usage limit).
    At(chrono::DateTime<Utc>),
}

pub async fn handle_research(svc: &Services, job: &Job) -> Result<Outcome, String> {
    let run_id = payload_id(job, "run_id")?;
    let Some(run) = research::load_run(&svc.pool, run_id).await.map_err(text)? else {
        return Ok(Outcome::Done);
    };
    if research::is_finished(&run.status) {
        return Ok(Outcome::Done);
    }
    research::begin_run(&svc.pool, run_id).await.map_err(text)?;
    let waited = Utc::now() - run.created_at;
    if waited < Duration::seconds(research::JUDGE_WAIT_SECS)
        && research::waiting_for_judge(&svc.pool, run.project_id)
            .await
            .map_err(text)?
    {
        return Ok(Outcome::RetryAt(Utc::now() + WAIT_FOR_JUDGE));
    }
    match work(svc, &run).await {
        Ok(()) => Ok(Outcome::Done),
        Err(Stop::At(at)) => Ok(Outcome::RetryAt(at)),
        Err(Stop::Fail(message)) => {
            research::fail_run(&svc.pool, run_id, &message)
                .await
                .map_err(text)?;
            Ok(Outcome::Done)
        }
        Err(Stop::Retry(message)) => {
            if job.attempts >= job.max_attempts {
                research::fail_run(&svc.pool, run_id, &message)
                    .await
                    .map_err(text)?;
            }
            Err(message)
        }
    }
}

async fn work(svc: &Services, run: &research::Run) -> Result<(), Stop> {
    let db = |e: openjam_core::Error| Stop::Retry(text(e));
    let ev = research::gather(&svc.pool, run.project_id)
        .await
        .map_err(db)?;
    if ev.answers_read == 0 {
        return Err(Stop::Fail(
            "no scan answers to read yet; run a scan first".into(),
        ));
    }
    let mut proposals = technical(svc, &ev).await;
    let mut read: HashMap<String, Fetched> = HashMap::new();
    for agent in [Agent::NewPages, Agent::PageEdits] {
        let (prompts, own) = match agent {
            Agent::NewPages => (ev.absent(), false),
            Agent::PageEdits => (ev.weak(), true),
        };
        let mut pages = Vec::new();
        for url in ev.pages_to_read(&prompts, own, PAGES_PER_AGENT) {
            let page = read_page(svc, &mut read, &url).await;
            pages.push(page);
        }
        let Some((system, user)) = research::agent_prompt(agent, &ev, &pages) else {
            continue;
        };
        let known: HashSet<_> = ev.prompts.iter().map(|p| p.id).collect();
        let mut found = ask_agent(svc, &ev, run, agent, &system, &user, &known).await?;
        if agent == Agent::PageEdits {
            for p in &mut found {
                let Some(url) = p.target_url.clone() else {
                    continue;
                };
                let page = read_page(svc, &mut read, &url).await;
                let baseline = page.ok().then(|| content_hash(&page.text));
                p.check = Some(CheckSpec::PageChanged { baseline });
            }
        }
        proposals.extend(found);
    }
    let stored = research::store_results(&svc.pool, run, &ev, proposals)
        .await
        .map_err(db)?;
    tracing::info!(
        run = %run.id,
        actions = stored.actions,
        opportunities = stored.opportunities,
        superseded = stored.superseded,
        "research finished"
    );
    Ok(())
}

/// One page as text, read once per run.
async fn read_page(svc: &Services, cache: &mut HashMap<String, Fetched>, url: &str) -> Fetched {
    let key = url_key(url);
    if let Some(page) = cache.get(&key) {
        return page.clone();
    }
    let mut page = svc.pages.read(url).await;
    page.text = crate::pages::html_to_text(&page.text);
    cache.insert(key, page.clone());
    page
}

async fn technical(svc: &Services, ev: &Evidence) -> Vec<Proposal> {
    let probe = site_probe(svc, &ev.site_url, true, true, true, true).await;
    technical_findings(&ev.site_url, &probe)
}

async fn site_probe(
    svc: &Services,
    site_url: &str,
    robots: bool,
    sitemap: bool,
    llms: bool,
    home: bool,
) -> Probe {
    let site = site_url.trim_end_matches('/');
    let get = |want: bool, path: &'static str| async move {
        if want {
            Some(svc.pages.read(&format!("{site}{path}")).await)
        } else {
            None
        }
    };
    Probe {
        robots: get(robots, "/robots.txt").await,
        sitemap: get(sitemap, "/sitemap.xml").await,
        llms: get(llms, "/llms.txt").await,
        home: get(home, "/").await,
        page: None,
    }
}

/// One model call that yields proposals. A reply that is not valid JSON repeats the call once;
/// after that the agent adds nothing and the run goes on.
async fn ask_agent(
    svc: &Services,
    ev: &Evidence,
    run: &research::Run,
    agent: Agent,
    system: &str,
    user: &str,
    known: &HashSet<uuid::Uuid>,
) -> Result<Vec<Proposal>, Stop> {
    for _ in 0..MODEL_TRIES {
        let routed = svc
            .router
            .complete(run.workspace_id, ModelTier::Big, system, user)
            .await
            .map_err(|e| match e {
                EngineError::RateLimited { retry_at, .. } => {
                    Stop::At(retry_at.unwrap_or(Utc::now() + Duration::minutes(5)))
                }
                EngineError::Timeout | EngineError::Unavailable(_) => Stop::Retry(e.to_string()),
                EngineError::CapReached => Stop::Fail("the spend cap is reached".into()),
                EngineError::NeedsKey | EngineError::NeedsLogin | EngineError::NotEligible => {
                    Stop::Fail(
                        "add an OpenAI or Claude account so research can read the answers".into(),
                    )
                }
                other => Stop::Fail(other.to_string()),
            })?;
        let done = &routed.value;
        usage::record(
            &svc.pool,
            &UsageEntry {
                workspace_id: run.workspace_id,
                project_id: Some(run.project_id),
                kind: "research",
                provider: None,
                model: Some(&done.model),
                backend_id: None,
                tokens_in: done.tokens_in,
                tokens_out: done.tokens_out,
                cost_cents: done.cost_cents.unwrap_or(0.0),
                reference: Some(&run.id.to_string()),
            },
        )
        .await
        .map_err(|e| Stop::Retry(text(e)))?;
        if let Some(found) = research::parse_proposals(&done.text, agent, known, &ev.domain) {
            return Ok(found);
        }
    }
    tracing::warn!(run = %run.id, ?agent, "the agent gave no valid JSON twice");
    Ok(vec![])
}

/// Start the free first run for projects that have scanned and never researched.
pub async fn tick(svc: &Services) -> Result<Vec<uuid::Uuid>, String> {
    research::start_first_runs(&svc.pool).await.map_err(text)
}

/// Check one done action: look at the page or site file and store whether the work is there.
pub async fn handle_check(svc: &Services, job: &Job) -> Result<Outcome, String> {
    let action_id = payload_id(job, "action_id")?;
    let Some(ctx) = actions::load_check(&svc.pool, action_id)
        .await
        .map_err(text)?
    else {
        return Ok(Outcome::Done);
    };
    let url = job.payload["url"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| ctx.target_url.clone());
    let site = &ctx.site_url;
    let probe = match &ctx.spec {
        CheckSpec::PageLive | CheckSpec::PageChanged { .. } => {
            let Some(url) = url.as_deref() else {
                return Ok(Outcome::Done);
            };
            let mut page = svc.pages.read(url).await;
            page.text = crate::pages::html_to_text(&page.text);
            Probe {
                page: Some(page),
                ..Probe::default()
            }
        }
        CheckSpec::RobotsAllowsAi => site_probe(svc, site, true, false, false, false).await,
        CheckSpec::Sitemap => site_probe(svc, site, true, true, false, false).await,
        CheckSpec::LlmsTxt => site_probe(svc, site, false, false, true, false).await,
        CheckSpec::Indexable => site_probe(svc, site, false, false, false, true).await,
    };
    let outcome = evaluate(&ctx.spec, &ctx.brand, &probe);
    actions::store_check(&svc.pool, &ctx, url.as_deref(), &outcome, Utc::now())
        .await
        .map_err(text)?;
    Ok(Outcome::Done)
}
