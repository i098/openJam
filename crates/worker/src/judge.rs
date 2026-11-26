//! The judge: read one stored answer with a model and store which brands it names.
//!
//! `unchecked`: the small and the big model both read each answer until the sample is full; the big
//! verdict is stored and the agreement is counted. Then `small_first` or `big_only` is fixed.

use crate::{Outcome, Services, payload_id, text};
use chrono::{DateTime, Duration, Utc};
use openjam_core::{
    jobs::Job,
    tracking::{self, AnswerContext},
    tracking_parse::{BrandSpec, JudgeMode, Verdict, judge_prompt, parse_verdict},
    usage::{self, UsageEntry},
};
use openjam_engines::{EngineError, router::ModelTier};

/// Why one judge call gave no verdict.
enum Stop {
    Retry(DateTime<Utc>),
    Fail(String),
    /// No account can judge, or the spend cap is reached: leave the answer unassessed.
    Skip(String),
}

pub async fn handle(svc: &Services, job: &Job) -> Result<Outcome, String> {
    let answer_id = payload_id(job, "answer_id")?;
    let Some(answer) = tracking::load_answer(&svc.pool, answer_id)
        .await
        .map_err(text)?
    else {
        return Ok(Outcome::Done);
    };
    if answer.assessed {
        return Ok(Outcome::Done);
    }
    match judge(svc, &answer).await {
        Ok(()) => Ok(Outcome::Done),
        Err(Stop::Retry(at)) => Ok(Outcome::RetryAt(at)),
        Err(Stop::Fail(message)) => Err(message),
        Err(Stop::Skip(reason)) => {
            tracing::warn!(answer = %answer.answer_id, %reason, "answer left unassessed");
            Ok(Outcome::Done)
        }
    }
}

async fn judge(svc: &Services, answer: &AnswerContext) -> Result<(), Stop> {
    let db = |e| Stop::Fail(text(e));
    let own = tracking::own_brand(&svc.pool, answer.project_id)
        .await
        .map_err(db)?;
    let competitors = tracking::competitors(&svc.pool, answer.project_id)
        .await
        .map_err(db)?;
    let (system, user) = judge_prompt(&answer.question, &answer.text, &own, &competitors);
    let ask = |tier| ask_verdict(svc, answer, tier, &system, &user);

    let verdict = match tracking::judge_mode(&svc.pool).await.map_err(db)? {
        JudgeMode::SmallFirst => ask(ModelTier::Small).await?,
        JudgeMode::BigOnly => ask(ModelTier::Big).await?,
        JudgeMode::Unchecked => {
            let small = ask(ModelTier::Small).await?;
            let big = ask(ModelTier::Big).await?;
            let agreed = matches!((&small, &big), (Some(s), Some(b)) if s.agrees_with(b, &own));
            if let Some(mode) = tracking::record_agreement(&svc.pool, agreed)
                .await
                .map_err(db)?
            {
                tracing::info!(mode = mode.as_str(), "judge check finished");
            }
            big
        }
    };
    // Invalid JSON twice: the answer stays unassessed and is not scored.
    let Some(verdict) = verdict else {
        return Err(Stop::Skip("the judge gave no valid JSON".into()));
    };
    store(svc, answer, &own, &competitors, &verdict)
        .await
        .map_err(db)
}

async fn store(
    svc: &Services,
    a: &AnswerContext,
    own: &BrandSpec,
    competitors: &[BrandSpec],
    v: &Verdict,
) -> openjam_core::Result<()> {
    tracking::store_verdict(&svc.pool, a, own, competitors, v).await
}

/// One verdict from one tier. The reply may be invalid JSON; the call repeats once then.
async fn ask_verdict(
    svc: &Services,
    a: &AnswerContext,
    tier: ModelTier,
    system: &str,
    user: &str,
) -> Result<Option<Verdict>, Stop> {
    for _ in 0..2 {
        let routed = svc
            .router
            .complete(a.workspace_id, tier, system, user)
            .await
            .map_err(|e| match e {
                EngineError::RateLimited { retry_at, .. } => {
                    Stop::Retry(retry_at.unwrap_or(Utc::now() + Duration::minutes(5)))
                }
                EngineError::Timeout | EngineError::Unavailable(_) => Stop::Fail(e.to_string()),
                other => Stop::Skip(other.to_string()),
            })?;
        let done = &routed.value;
        usage::record(
            &svc.pool,
            &UsageEntry {
                workspace_id: a.workspace_id,
                project_id: Some(a.project_id),
                kind: "judge",
                provider: None,
                model: Some(&done.model),
                backend_id: None,
                tokens_in: done.tokens_in,
                tokens_out: done.tokens_out,
                cost_cents: done.cost_cents.unwrap_or(0.0),
                reference: Some(&a.answer_id.to_string()),
            },
        )
        .await
        .map_err(|e| Stop::Fail(text(e)))?;
        if let Some(verdict) = parse_verdict(&done.text) {
            return Ok(Some(verdict));
        }
    }
    Ok(None)
}
