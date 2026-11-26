//! One scan call: ask one prompt on one platform through the engine router and store the answer.

use crate::{Outcome, Services, payload_id, text};
use chrono::{Duration, Utc};
use openjam_core::{
    jobs::Job,
    tracking::{self, NewAnswer},
    usage::{self, UsageEntry},
};
use openjam_engines::EngineAnswer;
use openjam_engines::{Backend, EngineError, EngineRequest, Provider, router::Routed};

/// A call that waited this long for a limit to end gives up.
const MAX_WAIT: Duration = Duration::hours(24);
/// The shortest wait after a limit, so a missing reset time does not spin.
const MIN_WAIT: Duration = Duration::seconds(30);
const DEFAULT_WAIT: Duration = Duration::minutes(5);

pub async fn handle(svc: &Services, job: &Job) -> Result<Outcome, String> {
    let call_id = payload_id(job, "call_id")?;
    let Some(call) = tracking::load_call(&svc.pool, call_id)
        .await
        .map_err(text)?
    else {
        return Ok(Outcome::Done);
    };
    if tracking::is_finished(&call.status) {
        return Ok(Outcome::Done);
    }
    tracking::begin_call(&svc.pool, call_id)
        .await
        .map_err(text)?;

    let provider = call.platform.parse::<Provider>();
    let backend = match provider {
        Ok(p) => Backend::for_workspace(&svc.pool, call.workspace_id, p)
            .await
            .map_err(text)?
            .filter(|b| b.enabled),
        Err(_) => None,
    };
    let Some(backend) = backend else {
        let message = format!("{} is off or has no account", call.platform);
        tracking::fail_call(&svc.pool, &call, "engine_needs_key", &message)
            .await
            .map_err(text)?;
        return Ok(Outcome::Done);
    };

    let request = EngineRequest {
        prompt: call.prompt.clone(),
        model: None,
        locale: call.locale.clone(),
    };
    match svc.router.ask(&backend, &request).await {
        Ok(routed) => {
            store(svc, &call, &backend, routed).await?;
            Ok(Outcome::Done)
        }
        Err(EngineError::RateLimited { retry_at, .. }) => {
            let now = Utc::now();
            if call.created_at + MAX_WAIT < now {
                tracking::fail_call(
                    &svc.pool,
                    &call,
                    "engine_limited",
                    "the account limit did not end in 24 hours",
                )
                .await
                .map_err(text)?;
                return Ok(Outcome::Done);
            }
            let until = retry_at.unwrap_or(now + DEFAULT_WAIT).max(now + MIN_WAIT);
            tracking::wait_call(&svc.pool, call_id, until, "engine_limited")
                .await
                .map_err(text)?;
            Ok(Outcome::RetryAt(until))
        }
        // The router already tried the account twice. Retry the job with back-off, then give up.
        Err(e @ (EngineError::Timeout | EngineError::Unavailable(_)))
            if job.attempts < job.max_attempts =>
        {
            Err(e.to_string())
        }
        Err(e) => {
            tracking::fail_call(&svc.pool, &call, e.code(), &e.to_string())
                .await
                .map_err(text)?;
            Ok(Outcome::Done)
        }
    }
}

async fn store(
    svc: &Services,
    call: &tracking::CallContext,
    backend: &Backend,
    routed: Routed<EngineAnswer>,
) -> Result<(), String> {
    let Routed {
        value: answer,
        mode,
        account_id,
    } = routed;
    let own = tracking::own_brand(&svc.pool, call.project_id)
        .await
        .map_err(text)?;
    let citations: Vec<(String, Option<String>)> = answer
        .citations
        .iter()
        .map(|c| (c.url.clone(), c.title.clone()))
        .collect();
    let cost = answer.cost_cents.unwrap_or(0.0);
    tracking::record_answer(
        &svc.pool,
        call,
        &own,
        &NewAnswer {
            text: &answer.text,
            citations: &citations,
            route: mode.route(),
            model: &answer.model,
            account_id: Some(account_id),
            raw: &answer.raw,
            cost_cents: cost,
        },
    )
    .await
    .map_err(text)?;
    usage::record(
        &svc.pool,
        &UsageEntry {
            workspace_id: call.workspace_id,
            project_id: Some(call.project_id),
            kind: "answer",
            provider: Some(backend.provider.as_str()),
            model: Some(&answer.model),
            backend_id: Some(backend.id),
            tokens_in: answer.tokens_in,
            tokens_out: answer.tokens_out,
            cost_cents: cost,
            reference: Some(&call.call_id.to_string()),
        },
    )
    .await
    .map_err(text)
}
