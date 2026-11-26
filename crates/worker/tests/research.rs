//! One research run on real scan output: scan, judge, research (three agents), then the checks of
//! two done actions. Mock engine and mock web; needs `DATABASE_URL`, each test gets its own database.

use async_trait::async_trait;
use openjam_core::{
    actions::{self, Fetched},
    research,
    tracking::{self, NewPrompt},
};
use openjam_engines::{
    Account, AccountMode, Citation, Completion, CompletionRequest, Engine, EngineAnswer,
    EngineError, EngineRequest, Provider, Router, Secret, pool::Pool, router::JudgeModels,
    secrets::Keyring,
};
use openjam_worker::{Services, pages::PageReader, research_tick, run_one, schedule_tick};
use sqlx::{PgPool, Row};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

const BOTH: &str = r#"{"brands":[{"name":"Acme CRM","recommended":true,"rank":1,"sentiment":"positive"},{"name":"Rival","recommended":false,"rank":2,"sentiment":"neutral"}]}"#;
const RIVAL_ONLY: &str =
    r#"{"brands":[{"name":"Rival","recommended":true,"rank":1,"sentiment":"positive"}]}"#;

struct Mock;

/// The id of the question line that holds `text`, from an agent prompt.
fn question_id(prompt: &str, text: &str) -> String {
    let line = prompt
        .lines()
        .find(|l| l.starts_with("- id ") && l.contains(text))
        .unwrap_or_else(|| panic!("no question {text} in the agent prompt"));
    line.split_whitespace().nth(2).unwrap().to_owned()
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
        _account: &Account,
        request: &EngineRequest,
    ) -> Result<EngineAnswer, EngineError> {
        let (text, url) = match request.prompt.as_str() {
            "best crm for small teams" => ("1. Rival.", "https://rival.example/best-crm"),
            "crm pricing comparison" => ("1. Acme CRM, 2. Rival.", "https://rival.example/pricing"),
            _ => ("1. Acme CRM, 2. Rival.", "https://www.acme.example/pricing"),
        };
        Ok(EngineAnswer {
            text: format!("For \"{}\": {text}", request.prompt),
            citations: vec![Citation {
                url: url.into(),
                title: None,
            }],
            model: "mock-model".into(),
            tokens_in: 10,
            tokens_out: 20,
            cost_cents: Some(1.0),
            raw: serde_json::json!({"mock": true}),
        })
    }
    async fn complete(
        &self,
        _account: &Account,
        request: &CompletionRequest,
    ) -> Result<Completion, EngineError> {
        let text = if request.system.contains("find new pages") {
            serde_json::json!({"proposals": [{
                "title": "Write: best CRM for small teams",
                "why": "Rival's guide is cited and Acme is missing.",
                "impact": "high",
                "prompt_ids": [question_id(&request.user, "best crm for small teams")],
                "source_urls": ["https://rival.example/best-crm"],
                "details": ["Cover price, setup time and support."],
                "topic": "crm for small teams", "persona": "owner", "intent": "compare",
                "angle": "Honest comparison",
                "outline": [{"heading": "What small teams need", "intent": "frame"}]
            }]})
            .to_string()
        } else if request.system.contains("edits to existing pages") {
            serde_json::json!({"proposals": [{
                "title": "Add a comparison table to the pricing page",
                "why": "Answers cite Rival's pricing, not Acme's.",
                "impact": "medium",
                "prompt_ids": [question_id(&request.user, "crm pricing comparison")],
                "target_url": "https://acme.example/pricing",
                "change_kind": "add_comparison_table",
                "details": ["Put the plans next to Rival's."]
            }]})
            .to_string()
        } else if request.user.contains("1. Acme CRM") {
            BOTH.to_owned()
        } else {
            RIVAL_ONLY.to_owned()
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

/// A fixed web: URL to status and body. Anything else is a 404.
struct Web(HashMap<&'static str, (u16, &'static str)>);

#[async_trait]
impl PageReader for Web {
    async fn read(&self, url: &str) -> Fetched {
        let (status, text) = self.0.get(url).copied().unwrap_or((404, ""));
        Fetched {
            url: url.into(),
            status: Some(status),
            text: text.into(),
        }
    }
}

fn web() -> Web {
    Web(HashMap::from([
        (
            "https://rival.example/best-crm",
            (200, "<h1>Best CRM</h1><p>Rival is easy.</p>"),
        ),
        (
            "https://acme.example/pricing",
            (200, "<h1>Pricing</h1><p>Acme CRM costs 10 a month.</p>"),
        ),
        (
            "https://acme.example/robots.txt",
            (200, "User-agent: GPTBot\nDisallow: /\n"),
        ),
        ("https://acme.example/sitemap.xml", (200, "<urlset/>")),
        (
            "https://acme.example/best-crm-for-small-teams",
            (
                200,
                "<h1>Best CRM</h1><p>Why Acme CRM fits small teams.</p>",
            ),
        ),
        (
            "https://acme.example/",
            (200, "<html><body>Acme CRM</body></html>"),
        ),
    ]))
}

async fn drain(svc: &Services) {
    while run_one(svc, "test").await.unwrap() {}
}

#[sqlx::test(migrations = "../../migrations")]
async fn research_reads_scan_answers_proposes_work_and_checks_it(pool: PgPool) {
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
    for text in [
        "best crm for small teams",
        "acme crm alternatives",
        "crm pricing comparison",
    ] {
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
    engines
        .add_key(
            workspace,
            Provider::Openai,
            "a",
            &Secret::new("sk-a".to_owned()),
        )
        .await
        .unwrap();
    let svc = Services {
        pool: pool.clone(),
        router: Router::new(engines, vec![Arc::new(Mock)], JudgeModels::default()),
        scan_cap_per_day: None,
        pages: Arc::new(web()),
    };

    // The scan and the judge come first; research has nothing to read before them.
    schedule_tick(&svc, chrono::Utc::now()).await.unwrap();
    drain(&svc).await;
    assert_eq!(
        research_tick(&svc).await.unwrap().len(),
        1,
        "the free first run starts"
    );
    assert!(
        research_tick(&svc).await.unwrap().is_empty(),
        "and only once"
    );
    drain(&svc).await;

    let run = sqlx::query("SELECT status, kind, answers_read, actions_proposed FROM research_runs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(run.get::<String, _>("status"), "done");
    assert_eq!(run.get::<String, _>("kind"), "first_scan");
    assert_eq!(run.get::<i32, _>("answers_read"), 3);
    assert!(run.get::<i32, _>("actions_proposed") >= 3);

    let kinds: Vec<(String, String)> =
        sqlx::query_as("SELECT kind, state FROM opportunities ORDER BY kind")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        kinds,
        [
            ("create_page".to_owned(), "open".to_owned()),
            ("update_page".to_owned(), "open".to_owned())
        ]
    );
    let robots: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM actions WHERE kind = 'technical' AND check_spec->>'type' = 'robots_allows_ai' AND status = 'suggested'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(robots, 1, "robots.txt blocks GPTBot");
    let llms: i64 =
        sqlx::query_scalar("SELECT count(*) FROM actions WHERE check_spec->>'type' = 'llms_txt'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(llms, 1, "llms.txt is missing");

    // Report two actions done. The new page is live and names the brand; the edited page is the same.
    let token = |kind: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT completion_token FROM actions WHERE kind = $1")
                .bind(kind)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    actions::complete_with_token(
        &pool,
        &token("new_page").await,
        Some("https://acme.example/best-crm-for-small-teams"),
        None,
    )
    .await
    .unwrap();
    actions::complete_with_token(&pool, &token("page_edit").await, None, Some("done"))
        .await
        .unwrap();
    drain(&svc).await;
    let status = |kind: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT status FROM actions WHERE kind = $1")
                .bind(kind)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(status("new_page").await, "verified");
    assert_eq!(
        status("page_edit").await,
        "unverified",
        "the page did not change since research read it"
    );

    // A second run keeps the done actions and adds no copy of anything.
    research::request_run(&pool, project, research::RunKind::Manual)
        .await
        .unwrap();
    drain(&svc).await;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM actions")
        .fetch_one(&pool)
        .await
        .unwrap();
    let first_total: i64 = sqlx::query_scalar("SELECT count(DISTINCT fingerprint) FROM actions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, first_total, "same findings update the same rows");
    assert_eq!(status("new_page").await, "verified");
}
