//! One real scan with a real OpenAI API key, end to end: web search, citations, both judge models.
//! It costs a few cents, so it is ignored. Run it with:
//!   OPENJAM_LIVE_OPENAI_KEY=sk-... DATABASE_URL=... cargo test -p openjam-worker --test live -- --ignored --nocapture

use chrono::Utc;
use openjam_core::tracking::{self, NewPrompt};
use openjam_engines::{
    Provider, Router, Secret, key::KeyEngine, pool::Pool, router::JudgeModels, secrets::Keyring,
};
use openjam_worker::{Services, run_one, schedule_tick};
use sqlx::{PgPool, Row};
use std::sync::Arc;

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs a real key and spends money"]
async fn one_live_scan_with_a_real_openai_key(pool: PgPool) {
    let key = std::env::var("OPENJAM_LIVE_OPENAI_KEY").expect("set OPENJAM_LIVE_OPENAI_KEY");
    let workspace: uuid::Uuid =
        sqlx::query_scalar("INSERT INTO workspaces (name) VALUES ('live') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    let project: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO projects (workspace_id, name, domain, site_url, status, platforms, next_scan_at)
         VALUES ($1, 'Ahrefs', 'ahrefs.com', 'https://ahrefs.com', 'active', '{openai}', now() - interval '1 minute')
         RETURNING id",
    )
    .bind(workspace)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO competitors (project_id, name, domain) VALUES ($1, 'Semrush', 'semrush.com')",
    )
    .bind(project)
    .execute(&pool)
    .await
    .unwrap();
    let prompt =
        "What are the best tools for tracking how often my brand appears in AI search answers?";
    tracking::add_prompt(
        &pool,
        project,
        &NewPrompt {
            text: prompt.into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let (keyring, _) = Keyring::generate();
    let engines = Pool::new(pool.clone(), Arc::new(keyring));
    engines
        .add_key(workspace, Provider::Openai, "live key", &Secret::new(key))
        .await
        .unwrap();
    let svc = Services {
        pool: pool.clone(),
        router: Router::new(
            engines,
            vec![Arc::new(KeyEngine::new(Provider::Openai, None))],
            JudgeModels::from_env(),
        ),
        scan_cap_per_day: None,
    };

    let runs = schedule_tick(&svc, Utc::now()).await.unwrap();
    assert_eq!(runs.len(), 1);
    while run_one(&svc, "live").await.unwrap() {}

    let run = sqlx::query("SELECT status, cost_cents::float8 AS cost FROM scan_runs WHERE id = $1")
        .bind(runs[0])
        .fetch_one(&pool)
        .await
        .unwrap();
    let a = sqlx::query(
        "SELECT text, model, route, assessed, brand_mentioned, brand_recommended, position, sentiment, own_domain_cited, id
         FROM answers",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    println!(
        "run status: {} cost_cents: {:.4}",
        run.get::<String, _>("status"),
        run.get::<f64, _>("cost")
    );
    println!(
        "answer: model={} route={} assessed={} mentioned={:?} recommended={:?} position={:?} sentiment={:?} own_domain_cited={:?}",
        a.get::<String, _>("model"),
        a.get::<String, _>("route"),
        a.get::<bool, _>("assessed"),
        a.get::<Option<bool>, _>("brand_mentioned"),
        a.get::<Option<bool>, _>("brand_recommended"),
        a.get::<Option<i32>, _>("position"),
        a.get::<Option<String>, _>("sentiment"),
        a.get::<Option<bool>, _>("own_domain_cited"),
    );
    let text: String = a.get("text");
    println!(
        "answer text (first 300 chars): {}",
        text.chars()
            .take(300)
            .collect::<String>()
            .replace('\n', " ")
    );
    let cites: Vec<(String,)> =
        sqlx::query_as("SELECT domain FROM answer_citations ORDER BY id LIMIT 8")
            .fetch_all(&pool)
            .await
            .unwrap();
    println!(
        "citation domains: {:?}",
        cites.iter().map(|c| c.0.as_str()).collect::<Vec<_>>()
    );
    let brands: Vec<(String, bool, bool, Option<i32>)> =
        sqlx::query_as("SELECT name, owned, recommended, list_rank FROM answer_brands ORDER BY list_rank NULLS LAST, name LIMIT 8")
            .fetch_all(&pool)
            .await
            .unwrap();
    println!("brands: {brands:?}");
    let ledger: Vec<(String, Option<String>, i32, i32, f64)> = sqlx::query_as(
        "SELECT kind, model, tokens_in, tokens_out, cost_cents::float8 FROM usage_ledger WHERE kind <> 'scan' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    println!("usage ledger (kind, model, in, out, cents): {ledger:?}");
    assert_eq!(run.get::<String, _>("status"), "complete");
    assert!(!cites.is_empty(), "a searched answer has sources");
}
