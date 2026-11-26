//! Analytics and answers on fixture answers. Every expected number is worked out by hand in the comments.
//! Needs `DATABASE_URL`.
//!
//! Fixture, project Acme (acme.com), competitors Rival (rival.com) and Other, `rangeDays` 30:
//!
//! | answer | prompt | engine | days ago | mentioned | cited | position | competitors (list rank)  | cited domains       |
//! |--------|--------|--------|----------|-----------|-------|----------|--------------------------|---------------------|
//! | a1     | P1     | openai | 1        | yes       | yes   | 1        | Rival (2)                | acme.com twice      |
//! | a2     | P1     | openai | 2        | no        | no    | -        | Rival (1), Other (2)     | rival.com           |
//! | a3     | P2     | openai | 2        | yes       | no    | 3        | -                        | g2.com              |
//! | a4     | P1     | claude | 3        | yes       | yes   | 2        | -                        | acme.com            |
//! | a5     | P2     | claude | 3        | no        | no    | -        | Other (1)                | -                   |
//! | b1     | P1     | openai | 40       | no        | no    | -        | Rival (1)                | -                   |
//! | b2     | P2     | claude | 40       | yes       | no    | 2        | -                        | -                   |
//!
//! Left out of every metric: an answer to an unscored prompt, an unassessed answer, an answer to an
//! archived prompt, and an answer 100 days ago (outside both windows).

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use chrono::{DateTime, Duration, Utc};
use http_body_util::BodyExt;
use openjam_api::{router, state::AppState};
use openjam_core::{auth::new_api_key, config::Config};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    app: Router,
    key: String,
    p1: Uuid,
    days: [String; 3],
}

async fn get(app: &Router, key: Option<&str>, uri: &str) -> (StatusCode, Value) {
    let mut request = Request::builder().uri(uri);
    if let Some(key) = key {
        request = request.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn workspace(pool: &PgPool, name: &str) -> (Uuid, String) {
    let workspace: Uuid =
        sqlx::query_scalar("INSERT INTO workspaces (name) VALUES ($1) RETURNING id")
            .bind(name)
            .fetch_one(pool)
            .await
            .unwrap();
    let user: Uuid = sqlx::query_scalar("INSERT INTO users (email) VALUES ($1) RETURNING id")
        .bind(format!("{name}@example.com"))
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO memberships (workspace_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(workspace)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    let key = new_api_key();
    sqlx::query("INSERT INTO api_keys (workspace_id, created_by, name, prefix, hash) VALUES ($1, $2, 'test', $3, $4)")
        .bind(workspace)
        .bind(user)
        .bind(&key.prefix)
        .bind(&key.hash)
        .execute(pool)
        .await
        .unwrap();
    (workspace, key.secret)
}

async fn prompt(
    pool: &PgPool,
    project: Uuid,
    text: &str,
    topic: &str,
    scored: bool,
    list: &str,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO prompts (project_id, text, topic, intent, scored, list) VALUES ($1, $2, $3, 'compare', $4, $5) RETURNING id",
    )
    .bind(project)
    .bind(text)
    .bind(topic)
    .bind(scored)
    .bind(list)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[derive(Default)]
struct Seed<'a> {
    platform: &'a str,
    days_ago: i64,
    text: String,
    assessed: bool,
    mentioned: bool,
    cited: bool,
    position: Option<i32>,
    brands: &'a [(&'a str, i32)],
    urls: &'a [&'a str],
}

async fn answer(pool: &PgPool, run: Uuid, prompt: Uuid, seed: Seed<'_>) -> DateTime<Utc> {
    let observed_at = Utc::now() - Duration::days(seed.days_ago);
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO answers (run_id, prompt_id, platform, observed_at, text, assessed, brand_mentioned, own_domain_cited, position)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
    )
    .bind(run)
    .bind(prompt)
    .bind(seed.platform)
    .bind(observed_at)
    .bind(&seed.text)
    .bind(seed.assessed)
    .bind(seed.assessed.then_some(seed.mentioned))
    .bind(seed.assessed.then_some(seed.cited))
    .bind(seed.position)
    .fetch_one(pool)
    .await
    .unwrap();
    for (name, rank) in seed.brands {
        sqlx::query("INSERT INTO answer_brands (answer_id, name, list_rank) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(name)
            .bind(rank)
            .execute(pool)
            .await
            .unwrap();
    }
    for url in seed.urls {
        let domain = url.split('/').next().unwrap();
        sqlx::query("INSERT INTO answer_citations (answer_id, url, domain) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(format!("https://{url}"))
            .bind(domain)
            .execute(pool)
            .await
            .unwrap();
    }
    observed_at
}

async fn fixture(pool: PgPool) -> Fixture {
    let (workspace, key) = workspace(&pool, "acme").await;
    let project: Uuid = sqlx::query_scalar(
        "INSERT INTO projects (workspace_id, name, domain, site_url) VALUES ($1, 'Acme', 'acme.com', 'https://acme.com') RETURNING id",
    )
    .bind(workspace)
    .fetch_one(&pool)
    .await
    .unwrap();
    for (name, domain) in [("Rival", Some("rival.com")), ("Other", None)] {
        sqlx::query("INSERT INTO competitors (project_id, name, domain) VALUES ($1, $2, $3)")
            .bind(project)
            .bind(name)
            .bind(domain)
            .execute(&pool)
            .await
            .unwrap();
    }
    let run: Uuid = sqlx::query_scalar(
        "INSERT INTO scan_runs (project_id, status) VALUES ($1, 'complete') RETURNING id",
    )
    .bind(project)
    .fetch_one(&pool)
    .await
    .unwrap();
    let p1 = prompt(&pool, project, "best crm tools", "crm", true, "tracked").await;
    let p2 = prompt(&pool, project, "crm pricing", "pricing", true, "tracked").await;
    let unscored = prompt(&pool, project, "acme crm review", "crm", false, "tracked").await;
    let archived = prompt(&pool, project, "old crm question", "crm", true, "archived").await;

    let base = || Seed {
        assessed: true,
        text: "short answer".into(),
        ..Seed::default()
    };
    let long = "x".repeat(400);
    let t1 = answer(
        &pool,
        run,
        p1,
        Seed {
            platform: "openai",
            days_ago: 1,
            text: long,
            mentioned: true,
            cited: true,
            position: Some(1),
            brands: &[("Rival", 2)],
            urls: &["acme.com/a", "acme.com/b"],
            ..base()
        },
    )
    .await;
    let t2 = answer(
        &pool,
        run,
        p1,
        Seed {
            platform: "openai",
            days_ago: 2,
            brands: &[("Rival", 1), ("Other", 2)],
            urls: &["rival.com/z"],
            ..base()
        },
    )
    .await;
    answer(
        &pool,
        run,
        p2,
        Seed {
            platform: "openai",
            days_ago: 2,
            mentioned: true,
            position: Some(3),
            urls: &["g2.com/crm"],
            ..base()
        },
    )
    .await;
    let t4 = answer(
        &pool,
        run,
        p1,
        Seed {
            platform: "claude",
            days_ago: 3,
            mentioned: true,
            cited: true,
            position: Some(2),
            urls: &["acme.com/c"],
            ..base()
        },
    )
    .await;
    answer(
        &pool,
        run,
        p2,
        Seed {
            platform: "claude",
            days_ago: 3,
            brands: &[("Other", 1)],
            ..base()
        },
    )
    .await;
    answer(
        &pool,
        run,
        p1,
        Seed {
            platform: "openai",
            days_ago: 40,
            brands: &[("Rival", 1)],
            ..base()
        },
    )
    .await;
    answer(
        &pool,
        run,
        p2,
        Seed {
            platform: "claude",
            days_ago: 40,
            mentioned: true,
            position: Some(2),
            ..base()
        },
    )
    .await;
    // Left out of every metric.
    answer(
        &pool,
        run,
        unscored,
        Seed {
            platform: "openai",
            days_ago: 1,
            mentioned: true,
            ..base()
        },
    )
    .await;
    answer(
        &pool,
        run,
        p1,
        Seed {
            platform: "openai",
            days_ago: 1,
            assessed: false,
            ..base()
        },
    )
    .await;
    answer(
        &pool,
        run,
        archived,
        Seed {
            platform: "openai",
            days_ago: 1,
            mentioned: true,
            ..base()
        },
    )
    .await;
    answer(
        &pool,
        run,
        p1,
        Seed {
            platform: "openai",
            days_ago: 100,
            mentioned: true,
            ..base()
        },
    )
    .await;

    let day = |t: DateTime<Utc>| t.format("%Y-%m-%d").to_string();
    Fixture {
        app: router(AppState {
            pool,
            config: Arc::new(Config {
                database_url: String::new(),
                bind: String::new(),
                allow_signup: true,
                cookie_secure: false,
                session_days: 30,
                scan_cap_per_day: None,
            }),
        }),
        key,
        p1,
        days: [day(t1), day(t2), day(t4)],
    }
}

async fn ok(f: &Fixture, uri: &str) -> Value {
    let (status, body) = get(&f.app, Some(&f.key), uri).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

fn row<'a>(rows: &'a Value, field: &str, value: &str) -> &'a Value {
    rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r[field] == value)
        .unwrap_or_else(|| panic!("no row with {field}={value}: {rows}"))
}

#[sqlx::test(migrations = "../../migrations")]
async fn overview_numbers(pool: PgPool) {
    let f = fixture(pool).await;
    let body = ok(&f, "/v1/analytics/overview?rangeDays=30").await;
    let o = &body["overview"];

    // Window: 5 answers. Mention rate 3/5 = 60, before 1/2 = 50.
    assert_eq!(body["window"]["answerCount"], 5);
    assert_eq!(body["window"]["providerCount"], 2);
    assert_eq!(
        o["visibilityScore"],
        serde_json::json!({"value": 60.0, "delta": 10.0, "deltaAvailable": true})
    );
    // Citation 2/5 = 40, before 0/2 = 0.
    assert_eq!(o["citationShare"]["value"], 40.0);
    assert_eq!(o["citationShare"]["delta"], 40.0);
    // Positions 1, 3, 2 give 2.0. Before: one position, 2.0.
    assert_eq!(o["averagePosition"]["value"], 2.0);
    assert_eq!(o["averagePosition"]["delta"], 0.0);
    // Own 3 mentions, Rival 2, Other 2: 3/7 = 42.86. Before: own 1, Rival 1: 50, so -7.14.
    assert_eq!(o["shareOfVoice"]["value"], 42.86);
    assert_eq!(o["shareOfVoice"]["delta"], -7.14);

    assert_eq!(body["trackedPrompts"], 2);
    assert_eq!(body["promptsMeasured"], 2);

    // Leaderboard: Acme 60. Rival and Other tie at 2/5 = 40 and share rank 2; the name breaks the tie.
    let board = o["leaderboard"].as_array().unwrap();
    let names: Vec<_> = board
        .iter()
        .map(|e| e["brand"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Acme", "Other", "Rival"]);
    assert_eq!(board[0]["rank"], 1);
    assert_eq!(board[1]["rank"], 2);
    assert_eq!(board[2]["rank"], 2);
    let rival = &board[2];
    assert_eq!(rival["visibilityScore"], 40.0);
    assert_eq!(rival["shareOfVoice"], 28.57);
    assert_eq!(rival["averagePosition"], 1.5);
    assert_eq!(
        rival["citationShare"], 20.0,
        "a2 cites rival.com: 1 of 5 answers"
    );
    assert_eq!(
        board[1]["citationShare"],
        Value::Null,
        "Other has no domain"
    );

    // Engines: openai 2/3 mention, claude 1/2; perplexity has no answers.
    let engines = o["providerBreakdown"].as_array().unwrap();
    let ids: Vec<_> = engines
        .iter()
        .map(|e| e["providerId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["openai", "perplexity", "claude"]);
    assert_eq!(engines[0]["visibilityScore"]["value"], 66.67);
    assert_eq!(engines[0]["visibilityScore"]["delta"], 66.67);
    assert_eq!(engines[0]["status"], "active");
    assert_eq!(engines[1]["status"], "pending");
    assert_eq!(engines[1]["visibilityScore"]["value"], Value::Null);
    assert_eq!(engines[2]["visibilityScore"]["delta"], -50.0);
    // Share of voice, openai: own 2, Rival 2, Other 1 gives 2/5 = 40.
    assert_eq!(engines[0]["shareOfVoice"]["value"], 40.0);

    // Biggest swing first: openai +66.67, claude -50.
    let feed = o["changeFeed"].as_array().unwrap();
    assert_eq!(feed[0]["kind"], "visibility_up");
    assert_eq!(feed[0]["providerId"], "openai");
    assert_eq!(feed[1]["kind"], "visibility_down");

    // Topics: crm 2/3 = 66.67, pricing 1/2 = 50.
    let topics = o["topicRankings"].as_array().unwrap();
    assert_eq!(topics[0]["topic"], "crm");
    assert_eq!(topics[0]["visibilityScore"]["value"], 66.67);
    assert_eq!(topics[1]["topic"], "pricing");
    assert_eq!(topics[1]["promptCount"], 1);

    // Trend: one point for each day with answers, own score and the top competitors.
    let points = o["trend"]["points"].as_array().unwrap();
    assert_eq!(points.len(), 3);
    let day3 = &points[0];
    assert_eq!(day3["date"], f.days[2]);
    assert_eq!(day3["scores"]["acme"], 50.0);
    assert_eq!(day3["scores"]["other"], 50.0);
    assert_eq!(day3["scores"]["rival"], 0.0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn overview_with_no_answers_has_no_values(pool: PgPool) {
    let f = fixture(pool).await;
    // Every fixture answer is at least one day old, and the window is measured from now.
    let body = ok(&f, "/v1/analytics/overview?rangeDays=1").await;
    let o = &body["overview"];
    assert_eq!(body["window"]["answerCount"], 0);
    assert_eq!(
        o["visibilityScore"],
        serde_json::json!({"value": null, "delta": null, "deltaAvailable": false})
    );
    assert_eq!(o["shareOfVoice"]["value"], Value::Null);
    assert_eq!(o["leaderboard"][0]["visibilityScore"], Value::Null);
    assert_eq!(o["leaderboard"][0]["rank"], Value::Null);
}

#[sqlx::test(migrations = "../../migrations")]
async fn prompt_metrics_and_paging(pool: PgPool) {
    let f = fixture(pool).await;
    let body = ok(&f, "/v1/analytics/prompts?rangeDays=30").await;
    let prompts = body["prompts"].as_array().unwrap();
    assert_eq!(
        prompts.len(),
        2,
        "unscored and archived prompts are left out"
    );
    // P1: 3 answers, 2 mention (66.67), 2 cited (66.67), positions 1 and 2 (1.5); own 2 against Rival 2 and Other 1 is 40.
    let p1 = &prompts[0]["metrics"];
    assert_eq!(prompts[0]["text"], "best crm tools");
    assert_eq!(p1["visibilityRank"], 1);
    assert_eq!(p1["citationRank"], 1);
    assert_eq!(p1["executions"], 3);
    assert_eq!(p1["visibilityScore"]["value"], 66.67);
    assert_eq!(p1["averagePosition"]["value"], 1.5);
    assert_eq!(p1["shareOfVoice"]["value"], 40.0);
    // P2: 2 answers, 1 mention (50), no citation (0), position 3; own 1 against Other 1 is 50.
    let p2 = &prompts[1]["metrics"];
    assert_eq!(p2["visibilityRank"], 2);
    assert_eq!(p2["citationShare"]["value"], 0.0);
    assert_eq!(p2["averagePosition"]["value"], 3.0);
    assert_eq!(p2["shareOfVoice"]["value"], 50.0);

    // One engine only: P1 on claude is one answer that mentions and cites.
    let claude = ok(&f, "/v1/analytics/prompts?platform=claude").await;
    assert_eq!(claude["prompts"][0]["metrics"]["executions"], 1);
    assert_eq!(
        claude["prompts"][0]["metrics"]["citationShare"]["value"],
        100.0
    );
    // A text filter that matches nothing, with `%` taken as text.
    let none = ok(&f, "/v1/analytics/prompts?query=%25").await;
    assert_eq!(none["pageInfo"]["totalCount"], 0);

    // Paging: one at a time, then the end.
    let first = ok(&f, "/v1/analytics/prompts?limit=1").await;
    assert_eq!(first["pageInfo"]["hasNextPage"], true);
    let cursor = first["pageInfo"]["nextCursor"].as_str().unwrap();
    let second = ok(
        &f,
        &format!("/v1/analytics/prompts?limit=1&cursor={cursor}"),
    )
    .await;
    assert_eq!(second["prompts"][0]["text"], "crm pricing");
    assert_eq!(second["pageInfo"]["hasNextPage"], false);
    assert_eq!(second["pageInfo"]["nextCursor"], Value::Null);
}

#[sqlx::test(migrations = "../../migrations")]
async fn metric_families(pool: PgPool) {
    let f = fixture(pool).await;

    // Citations by day: day 1 (1 of 1), day 2 (0 of 2), day 3 (1 of 2).
    let citations = ok(&f, "/v1/analytics/metrics?metric=citations").await;
    assert_eq!(citations["metric"], "citations");
    assert_eq!(row(&citations, "date", &f.days[0])["citationShare"], 100.0);
    assert_eq!(row(&citations, "date", &f.days[1])["citationShare"], 0.0);
    let day3 = row(&citations, "date", &f.days[2]);
    assert_eq!(
        (
            &day3["citationShare"],
            &day3["citations"],
            &day3["answerCount"]
        ),
        (&Value::from(50.0), &Value::from(1), &Value::from(2))
    );

    // Visibility on claude only: day 3 has a4 (mentions) and a5 (Other 1st). Own 1/2 = 50, SoV 1 of 2 = 50.
    let visibility = ok(
        &f,
        "/v1/analytics/metrics?metric=visibility&platform=claude",
    )
    .await;
    let rows = visibility["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["brand"], "Acme");
    assert_eq!(rows[0]["visibilityScore"], 50.0);
    assert_eq!(rows[0]["shareOfVoice"], 50.0);
    assert_eq!(rows[0]["averagePosition"], 2.0);
    assert_eq!(rows[1]["brand"], "Other");
    assert_eq!(rows[1]["mentions"], 1);

    // Competitors ranking.
    let competitors = ok(&f, "/v1/analytics/metrics?metric=competitors").await;
    let acme = row(&competitors, "brand", "Acme");
    assert_eq!(
        (&acme["rank"], &acme["mentions"], &acme["isOwned"]),
        (&Value::from(1), &Value::from(3), &Value::from(true))
    );

    // Engines.
    let platforms = ok(&f, "/v1/analytics/metrics?metric=platforms").await;
    let claude = row(&platforms, "platform", "claude");
    assert_eq!(claude["visibilityScore"], 50.0);
    assert_eq!(claude["citationShare"], 50.0);
    assert_eq!(claude["answerCount"], 2);
    assert_eq!(
        row(&platforms, "platform", "perplexity")["status"],
        "pending"
    );

    // Topics.
    let topics = ok(&f, "/v1/analytics/metrics?metric=topics").await;
    assert_eq!(row(&topics, "topic", "crm")["visibilityScore"], 66.67);
    assert_eq!(row(&topics, "topic", "pricing")["answerCount"], 2);

    // /v1/keywords is the topics metric under its own name.
    assert_eq!(ok(&f, "/v1/keywords?rangeDays=30").await, topics);

    // Sources: acme.com 3 citations in 2 answers of 5 (40). Own domain is flagged.
    let sources = ok(&f, "/v1/analytics/metrics?metric=sources").await;
    let acme = &sources["rows"][0];
    assert_eq!(acme["domain"], "acme.com");
    assert_eq!(
        (
            &acme["citations"],
            &acme["answers"],
            &acme["answerShare"],
            &acme["isOwned"]
        ),
        (
            &Value::from(3),
            &Value::from(2),
            &Value::from(40.0),
            &Value::from(true)
        )
    );
    assert_eq!(row(&sources, "domain", "rival.com")["isOwned"], false);
}

#[sqlx::test(migrations = "../../migrations")]
async fn answers_filter_and_page(pool: PgPool) {
    let f = fixture(pool).await;

    // All 11 stored answers, 4 a page, newest first, no repeats.
    let mut seen = Vec::new();
    let mut uri = "/v1/answers?limit=4".to_string();
    let mut previous: Option<String> = None;
    loop {
        let page = ok(&f, &uri).await;
        for a in page["answers"].as_array().unwrap() {
            let at = a["observedAt"].as_str().unwrap().to_string();
            if let Some(before) = &previous {
                assert!(at <= *before, "newest first");
            }
            previous = Some(at);
            seen.push(a["id"].as_str().unwrap().to_string());
        }
        match page["nextCursor"].as_str() {
            Some(cursor) => uri = format!("/v1/answers?limit=4&cursor={cursor}"),
            None => break,
        }
    }
    assert_eq!(seen.len(), 11);
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 11);

    // Filters.
    assert_eq!(
        ok(&f, "/v1/answers?cited=true").await["answers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        ok(&f, "/v1/answers?competitor=rival").await["answers"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        ok(&f, "/v1/answers?platform=claude").await["answers"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let since =
        (Utc::now() - Duration::days(10)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let recent = ok(&f, &format!("/v1/answers?since={since}&platform=claude")).await;
    assert_eq!(recent["answers"].as_array().unwrap().len(), 2);

    // Excerpt by default, whole text with full=true. a1 has 400 characters, two cited URLs and one competitor.
    let a1 = ok(&f, "/v1/answers?cited=true&platform=openai").await["answers"][0].clone();
    assert_eq!(a1["answer"]["source"], "excerpt");
    assert_eq!(a1["answer"]["truncated"], true);
    assert_eq!(a1["answer"]["text"].as_str().unwrap().len(), 280);
    assert_eq!(a1["answer"]["length"], 400);
    assert_eq!(a1["citedUrlCount"], 2);
    assert_eq!(a1["brandsMentioned"][0]["name"], "Rival");
    assert_eq!(a1["brandsMentioned"][0]["listRank"], 2);
    assert_eq!(a1["position"], 1);
    let full =
        ok(&f, "/v1/answers?cited=true&platform=openai&full=true").await["answers"][0].clone();
    assert_eq!(full["answer"]["source"], "full");
    assert_eq!(full["answer"]["text"].as_str().unwrap().len(), 400);

    // One prompt: P1 has a1, a2, a4, b1, one unassessed and one 100 days old.
    let p1 = ok(&f, &format!("/v1/prompts/{}/answers", f.p1)).await;
    assert_eq!(p1["answers"].as_array().unwrap().len(), 6);
    assert_eq!(p1["pageInfo"]["hasNextPage"], false);
    assert_eq!(
        ok(&f, &format!("/v1/answers?promptId={}", f.p1)).await["answers"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn rejects_bad_requests_and_other_workspaces(pool: PgPool) {
    let f = fixture(pool.clone()).await;
    for uri in [
        "/v1/analytics/overview",
        "/v1/analytics/prompts",
        "/v1/analytics/metrics?metric=visibility",
        "/v1/answers",
    ] {
        let (status, _) = get(&f.app, None, uri).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }
    for uri in [
        "/v1/analytics/overview?rangeDays=0",
        "/v1/analytics/overview?rangeDays=366",
        "/v1/analytics/overview?rangeDays=abc",
        "/v1/analytics/prompts?limit=0",
        "/v1/analytics/prompts?limit=101",
        "/v1/analytics/prompts?cursor=abc",
        "/v1/analytics/metrics",
        "/v1/analytics/metrics?metric=bogus",
        "/v1/answers?limit=101",
        "/v1/answers?cursor=1_notauuid",
        "/v1/answers?since=yesterday",
    ] {
        let (status, body) = get(&f.app, Some(&f.key), uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        assert_eq!(body["code"], "invalid_request", "{uri}: {body}");
    }

    // Another workspace sees no project and none of these answers.
    let (_, other) = workspace(&pool, "other").await;
    let (status, _) = get(&f.app, Some(&other), "/v1/analytics/overview").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = get(&f.app, Some(&other), "/v1/answers").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["answers"].as_array().unwrap().len(), 0);
    let (status, _) = get(
        &f.app,
        Some(&other),
        &format!("/v1/prompts/{}/answers", f.p1),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
