//! End to end through a mock engine: the scheduler starts a scan, the worker asks the engine,
//! rotates accounts on a usage limit, stores answers and citations, and the judge assesses them.
//! Needs `DATABASE_URL`; each test gets its own database.

use async_trait::async_trait;
use chrono::{Duration, Utc};
use openjam_core::tracking::{self, NewPrompt};
use openjam_engines::{
    Account, AccountMode, Citation, Completion, CompletionRequest, Engine, EngineAnswer,
    EngineError, EngineRequest, Provider, Router, Secret, pool::Pool, router::JudgeModels,
    secrets::Keyring,
};
use openjam_worker::{Services, run_one, schedule_tick};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;

const VERDICT: &str = r#"{"brands":[{"name":"Acme CRM","recommended":true,"rank":1,"sentiment":"positive"},{"name":"Rival","recommended":false,"rank":2,"sentiment":"neutral"}]}"#;
const SMALL_MODEL: &str = "gpt-5-nano";

struct Mock {
    /// This account is always at its usage limit.
    limited_label: Option<&'static str>,
    /// The small judge model leaves out a brand the big one finds.
    small_disagrees: bool,
    /// The judge replies with text that is not JSON.
    judge_garbage: bool,
}

#[async_trait]
impl Engine for Mock {
    fn provider(&self) -> Provider {
        Provider::Openai
    }
    fn mode(&self) -> AccountMode {
        AccountMode::Key
    }
    async fn run(
        &self,
        account: &Account,
        request: &EngineRequest,
    ) -> Result<EngineAnswer, EngineError> {
        if self.limited_label == Some(account.label.as_str()) {
            return Err(EngineError::RateLimited {
                retry_at: Some(Utc::now() + Duration::hours(1)),
                usage_limit: true,
            });
        }
        Ok(EngineAnswer {
            text: format!("For \"{}\": 1. Acme CRM, 2. Rival.", request.prompt),
            citations: vec![Citation {
                url: "https://www.acme.example/pricing".into(),
                title: Some("Pricing".into()),
            }],
            model: "mock-model".into(),
            tokens_in: 10,
            tokens_out: 20,
            cost_cents: Some(1.5),
            raw: serde_json::json!({"mock": true}),
        })
    }
    async fn complete(
        &self,
        _account: &Account,
        request: &CompletionRequest,
    ) -> Result<Completion, EngineError> {
        let text = if self.judge_garbage {
            "sorry, no JSON".to_owned()
        } else if self.small_disagrees && request.model == SMALL_MODEL {
            r#"{"brands":[{"name":"Acme CRM","recommended":true,"rank":1,"sentiment":"positive"}]}"#
                .to_owned()
        } else {
            VERDICT.to_owned()
        };
        Ok(Completion {
            text,
            tokens_in: 5,
            tokens_out: 5,
            cost_cents: Some(0.1),
            model: request.model.clone(),
        })
    }
}

struct World {
    svc: Services,
    project: Uuid,
    workspace: Uuid,
    pool: PgPool,
}

async fn world(pool: PgPool, mock: Mock, accounts: &[&str]) -> World {
    let workspace: Uuid =
        sqlx::query_scalar("INSERT INTO workspaces (name) VALUES ('w') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    let project: Uuid = sqlx::query_scalar(
        "INSERT INTO projects (workspace_id, name, domain, site_url, status, platforms, cadence, next_scan_at)
         VALUES ($1, 'Acme CRM', 'acme.example', 'https://acme.example', 'active', '{openai}', 'daily', now() - interval '1 minute')
         RETURNING id",
    )
    .bind(workspace)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO competitors (project_id, name, domain) VALUES ($1, 'Rival', 'rival.example')",
    )
    .bind(project)
    .execute(&pool)
    .await
    .unwrap();
    for text in ["best crm for small teams", "acme crm alternatives"] {
        tracking::add_prompt(
            &pool,
            project,
            &NewPrompt {
                text: text.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    let (keyring, _) = Keyring::generate();
    let engines = Pool::new(pool.clone(), Arc::new(keyring));
    for (i, label) in accounts.iter().enumerate() {
        let id = engines
            .add_key(
                workspace,
                Provider::Openai,
                label,
                &Secret::new(format!("sk-{label}")),
            )
            .await
            .unwrap();
        // The first account has the highest priority, so it is tried first.
        sqlx::query("UPDATE engine_accounts SET priority = $2 WHERE id = $1")
            .bind(id)
            .bind(10 - i as i32)
            .execute(&pool)
            .await
            .unwrap();
    }
    let router = Router::new(engines, vec![Arc::new(mock)], JudgeModels::default());
    World {
        svc: Services {
            pool: pool.clone(),
            router,
            scan_cap_per_day: None,
        },
        project,
        workspace,
        pool,
    }
}

async fn drain(w: &World) {
    while run_one(&w.svc, "test").await.unwrap() {}
}

fn mock() -> Mock {
    Mock {
        limited_label: None,
        small_disagrees: false,
        judge_garbage: false,
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_scheduled_scan_stores_assessed_answers_and_moves_to_the_next_account_on_a_limit(
    pool: PgPool,
) {
    let w = world(
        pool,
        Mock {
            limited_label: Some("a"),
            ..mock()
        },
        &["a", "b"],
    )
    .await;

    let runs = schedule_tick(&w.svc, Utc::now()).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert!(
        schedule_tick(&w.svc, Utc::now()).await.unwrap().is_empty(),
        "the project is not due again"
    );
    let next: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT next_scan_at FROM projects WHERE id = $1")
            .bind(w.project)
            .fetch_one(&w.pool)
            .await
            .unwrap();
    assert!(next > Utc::now() + Duration::hours(23), "daily cadence");
    drain(&w).await;

    let run = sqlx::query(
        "SELECT status, prompt_count, cost_cents::float8 AS cost FROM scan_runs WHERE id = $1",
    )
    .bind(runs[0])
    .fetch_one(&w.pool)
    .await
    .unwrap();
    assert_eq!(run.get::<String, _>("status"), "complete");
    assert_eq!(run.get::<f64, _>("cost"), 3.0, "two answers at 1.5 cents");

    let answers = sqlx::query(
        "SELECT a.assessed, a.brand_mentioned, a.brand_recommended, a.own_domain_cited, a.position, a.sentiment, a.route, e.label
         FROM answers a JOIN engine_accounts e ON e.id = a.account_id",
    )
    .fetch_all(&w.pool)
    .await
    .unwrap();
    assert_eq!(answers.len(), 2);
    for a in &answers {
        assert!(
            a.get::<bool, _>("assessed")
                && a.get::<bool, _>("brand_mentioned")
                && a.get::<bool, _>("brand_recommended")
        );
        assert!(a.get::<bool, _>("own_domain_cited"));
        assert_eq!(
            (
                a.get::<Option<i32>, _>("position"),
                a.get::<Option<String>, _>("sentiment").as_deref()
            ),
            (Some(1), Some("positive"))
        );
        assert_eq!(
            (
                a.get::<String, _>("route").as_str(),
                a.get::<String, _>("label").as_str()
            ),
            ("api_key", "b"),
            "rotated off the limited account"
        );
    }
    let blocks: i64 = sqlx::query_scalar("SELECT count(*) FROM engine_account_blocks b JOIN engine_accounts e ON e.id = b.account_id WHERE e.label = 'a' AND b.scope = ''")
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(blocks, 1, "a usage limit blocks the whole account once");
    let brands: Vec<(String, bool)> =
        sqlx::query_as("SELECT DISTINCT name, owned FROM answer_brands ORDER BY name")
            .fetch_all(&w.pool)
            .await
            .unwrap();
    assert_eq!(
        brands,
        [("Acme CRM".to_owned(), true), ("Rival".to_owned(), false)]
    );
    let cites: i64 =
        sqlx::query_scalar("SELECT count(*) FROM answer_citations WHERE domain = 'acme.example'")
            .fetch_one(&w.pool)
            .await
            .unwrap();
    assert_eq!(cites, 2);
    let spent: f64 = sqlx::query_scalar("SELECT COALESCE(sum(cost_cents), 0)::float8 FROM usage_ledger WHERE workspace_id = $1 AND kind = 'answer'")
        .bind(w.workspace)
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(spent, 3.0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn standby_waits_for_the_limit_to_end_instead_of_using_the_next_account(pool: PgPool) {
    let w = world(
        pool,
        Mock {
            limited_label: Some("a"),
            ..mock()
        },
        &["a", "b"],
    )
    .await;
    sqlx::query("UPDATE engine_backends SET rotation = 'standby'")
        .execute(&w.pool)
        .await
        .unwrap();

    schedule_tick(&w.svc, Utc::now()).await.unwrap();
    drain(&w).await;

    let calls: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT status, error_code FROM scan_calls")
            .fetch_all(&w.pool)
            .await
            .unwrap();
    assert!(
        calls
            .iter()
            .all(|(s, c)| s == "waiting" && c.as_deref() == Some("engine_limited")),
        "{calls:?}"
    );
    let answers: i64 = sqlx::query_scalar("SELECT count(*) FROM answers")
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(answers, 0);
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE queue = 'scan' AND status = 'queued' AND run_at > now() + interval '30 minutes'")
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(queued, 2, "each call is set to run again after the limit");
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_agreement_check_picks_the_judge_mode_after_200_answers(pool: PgPool) {
    // 199 answers checked, all agreed; the first answer of this scan fills the sample.
    let w = world(pool, mock(), &["a"]).await;
    sqlx::query("UPDATE judge_state SET samples = 199, agreed = 199")
        .execute(&w.pool)
        .await
        .unwrap();
    schedule_tick(&w.svc, Utc::now()).await.unwrap();
    drain(&w).await;
    let state = sqlx::query("SELECT mode, samples FROM judge_state")
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(
        (
            state.get::<String, _>("mode").as_str(),
            state.get::<i32, _>("samples")
        ),
        ("small_first", 200),
        "the check stops counting at the decision"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_small_judge_that_disagrees_with_the_big_one_leads_to_big_only(pool: PgPool) {
    // The small model leaves out a brand, so no answer agrees: 100 of 200 is below the bar.
    let w = world(
        pool,
        Mock {
            small_disagrees: true,
            ..mock()
        },
        &["a"],
    )
    .await;
    sqlx::query("UPDATE judge_state SET samples = 199, agreed = 100")
        .execute(&w.pool)
        .await
        .unwrap();
    schedule_tick(&w.svc, Utc::now()).await.unwrap();
    drain(&w).await;
    let mode: String = sqlx::query_scalar("SELECT mode FROM judge_state")
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(mode, "big_only");
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_answer_the_judge_cannot_read_stays_unassessed(pool: PgPool) {
    let w = world(
        pool,
        Mock {
            judge_garbage: true,
            ..mock()
        },
        &["a"],
    )
    .await;
    schedule_tick(&w.svc, Utc::now()).await.unwrap();
    drain(&w).await;
    let status: String = sqlx::query_scalar("SELECT status FROM scan_runs")
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(status, "complete", "the scan itself succeeded");
    let assessed: i64 = sqlx::query_scalar("SELECT count(*) FROM answers WHERE assessed")
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(assessed, 0);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM answer_brands")
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "an unassessed answer is not scored");
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_optional_daily_cap_stops_a_second_scan_and_the_default_has_no_cap(pool: PgPool) {
    let w = world(pool, mock(), &["a"]).await;
    for _ in 0..3 {
        assert!(
            tracking::start_scan(&w.pool, w.project, None)
                .await
                .unwrap()
                .is_some(),
            "no cap by default"
        );
    }
    let capped = tracking::start_scan(&w.pool, w.project, Some(3)).await;
    assert!(matches!(
        capped,
        Err(openjam_core::Error::CapReached { cap: 3 })
    ));
}
