//! Content, revisions, Markdown export and the GitHub pull request flow, against a fake GitHub.
//! Needs `DATABASE_URL`.

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode, header},
    routing::{get, patch, post},
};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey};
use openjam_api::{router, state::AppState};
use openjam_core::config::Config;
use openjam_publish::{
    delivery::{self, Step},
    github::App,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

/// What the fake GitHub saw and what it answers for the pull request.
#[derive(Default)]
struct Hub {
    files: Vec<(String, String)>,
    merged: bool,
    closed: bool,
}

type Shared = Arc<Mutex<Hub>>;

async fn fake_github(hub: Shared) -> String {
    let app = Router::new()
        .route(
            "/repos/acme/site/installation",
            get(|| async {
                Json(json!({"id": 7, "permissions": {"contents": "write", "pull_requests": "write"}}))
            }),
        )
        .route(
            "/app/installations/7/access_tokens",
            post(|| async { Json(json!({"token": "t"})) }),
        )
        .route(
            "/repos/acme/site/git/ref/heads/main",
            get(|| async { Json(json!({"object": {"sha": "abc"}})) }),
        )
        .route(
            "/repos/acme/site/git/refs",
            post(|| async { (StatusCode::CREATED, Json(json!({}))) }),
        )
        .route(
            "/repos/acme/site/contents/{*path}",
            get(|| async { (StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"}))) })
                .put(
                    |State(hub): State<Shared>,
                     axum::extract::Path(path): axum::extract::Path<String>,
                     Json(body): Json<Value>| async move {
                        use base64::{Engine, engine::general_purpose::STANDARD};
                        let text = STANDARD.decode(body["content"].as_str().unwrap()).unwrap();
                        hub.lock()
                            .unwrap()
                            .files
                            .push((path, String::from_utf8(text).unwrap()));
                        (StatusCode::CREATED, Json(json!({})))
                    },
                ),
        )
        .route(
            "/repos/acme/site/pulls",
            post(|| async {
                (
                    StatusCode::CREATED,
                    Json(json!({"number": 3, "html_url": "https://github.com/acme/site/pull/3", "state": "open"})),
                )
            }),
        )
        .route(
            "/repos/acme/site/pulls/3",
            get(|State(hub): State<Shared>| async move {
                let hub = hub.lock().unwrap();
                Json(json!({
                    "number": 3,
                    "html_url": "https://github.com/acme/site/pull/3",
                    "state": if hub.merged || hub.closed { "closed" } else { "open" },
                    "merged": hub.merged,
                }))
            })
            .merge(patch(|State(hub): State<Shared>| async move {
                hub.lock().unwrap().closed = true;
                Json(json!({}))
            })),
        )
        .route(
            "/repos/acme/site/git/refs/heads/{*branch}",
            axum::routing::delete(|| async { StatusCode::NO_CONTENT }),
        )
        .with_state(hub);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

struct Client {
    app: Router,
    key: String,
}

impl Client {
    async fn call(
        &self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.key));
        let body = match body {
            Some(b) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(b.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::Null),
            text,
        )
    }
}

fn article(slug: &str, body: &str) -> Value {
    json!({
        "title": "How GEO works",
        "metaDescription": "A short guide.",
        "slug": slug,
        "keywords": ["geo", "ai search"],
        "bodyMarkdown": body,
    })
}

async fn setup(pool: PgPool, github: Option<Arc<App>>) -> (Client, Uuid) {
    let config = Config {
        database_url: String::new(),
        bind: String::new(),
        allow_signup: true,
        cookie_secure: false,
        session_days: 30,
        scan_cap_per_day: None,
    };
    let app = router(AppState {
        pool: pool.clone(),
        config: Arc::new(config),
        github,
    });
    let signup = Request::builder()
        .method("POST")
        .uri("/auth/signup")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"email": "a@b.co", "password": "hunter22"}).to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(signup).await.unwrap();
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let request = Request::builder()
        .method("POST")
        .uri("/v1/api-keys")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .body(Body::from(json!({"name": "k"}).to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let key = serde_json::from_slice::<Value>(&bytes).unwrap()["key"]
        .as_str()
        .unwrap()
        .to_owned();
    let client = Client { app, key };
    let (status, project, _) = client
        .call(
            "POST",
            "/v1/projects",
            Some(json!({"name": "Acme", "domain": "https://acme.com"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project: Uuid = project["id"].as_str().unwrap().parse().unwrap();
    (client, project)
}

async fn opportunity(pool: &PgPool, project: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO opportunities (project_id, kind, title) VALUES ($1, 'create_page', 'GEO') RETURNING id",
    )
    .bind(project)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn content_revisions_discard_and_markdown(pool: PgPool) {
    let (client, project) = setup(pool.clone(), None).await;
    let opp = opportunity(&pool, project).await;

    let (status, body, _) = client
        .call(
            "POST",
            "/v1/content",
            Some(json!({"opportunityId": opp, "article": article("Bad Slug", "## A\n\ntext")})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, created, _) = client
        .call(
            "POST",
            "/v1/content",
            Some(
                json!({"opportunityId": opp, "article": article("how-geo-works", "## A\n\ntext")}),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["contentId"].as_str().unwrap().to_owned();
    let first = created["revisionId"].as_str().unwrap().to_owned();

    // One live content per opportunity.
    let (status, _, _) = client
        .call(
            "POST",
            "/v1/content",
            Some(json!({"opportunityId": opp, "article": article("other-slug", "## A\n\ntext")})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, added, _) = client
        .call(
            "POST",
            &format!("/v1/content/{id}/revisions"),
            Some(json!({"article": article("how-geo-works", "## B\n\nnew text"), "basisRevisionId": first})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{added}");
    let second = added["revisionId"].as_str().unwrap().to_owned();

    let (_, revisions, _) = client
        .call("GET", &format!("/v1/content/{id}/revisions"), None)
        .await;
    assert_eq!(revisions["revisions"].as_array().unwrap().len(), 2);

    let (_, content, _) = client.call("GET", &format!("/v1/content/{id}"), None).await;
    assert_eq!(content["currentRevision"]["id"], second.as_str());

    let (status, _, markdown) = client
        .call("GET", &format!("/v1/content/{id}/markdown"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        markdown.starts_with("---\ntitle: \"How GEO works\""),
        "{markdown}"
    );
    assert!(markdown.contains("slug: \"how-geo-works\""), "{markdown}");
    assert!(markdown.ends_with("## B\n\nnew text\n"), "{markdown}");
    let (_, old, _) = client
        .call(
            "GET",
            &format!("/v1/content/{id}/markdown?revisionId={first}"),
            None,
        )
        .await;
    assert!(old.is_null());

    let (status, _, _) = client
        .call("DELETE", &format!("/v1/content/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, drafted, _) = client.call("GET", "/v1/content?state=drafted", None).await;
    assert_eq!(drafted["content"].as_array().unwrap().len(), 0);
    let (_, discarded, _) = client
        .call("GET", "/v1/content?state=discarded", None)
        .await;
    assert_eq!(discarded["pageInfo"]["totalCount"], 1);
    let (status, _, _) = client
        .call("POST", &format!("/v1/content/{id}/restorations"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, content, _) = client.call("GET", &format!("/v1/content/{id}"), None).await;
    assert_eq!(content["content"]["state"], "drafted");

    // A rewrite request is accepted and queued for the worker.
    let (status, _, _) = client
        .call(
            "POST",
            &format!("/v1/content/{id}/edit-requests"),
            Some(json!({"basisRevisionId": second, "instruction": "Shorter."})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let queued: i64 =
        sqlx::query_scalar("SELECT count(*) FROM jobs WHERE queue = 'content_update'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(queued, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn github_pull_request_flow(pool: PgPool) {
    let hub = Shared::default();
    let url = fake_github(hub.clone()).await;
    let github = Arc::new(App::new(
        "1".into(),
        EncodingKey::from_secret(b"test"),
        Algorithm::HS256,
        &url,
    ));
    let (client, project) = setup(pool.clone(), Some(github.clone())).await;
    let opp = opportunity(&pool, project).await;
    let (_, created, _) = client
        .call(
            "POST",
            "/v1/content",
            Some(
                json!({"opportunityId": opp, "article": article("how-geo-works", "## A\n\ntext")}),
            ),
        )
        .await;
    let id = created["contentId"].as_str().unwrap().to_owned();

    let (status, dest, _) = client
        .call(
            "POST",
            "/v1/destinations",
            Some(json!({"platform": "github"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{dest}");
    let dest = dest["destinationId"].as_str().unwrap().to_owned();

    // Publishing before the repository is connected is refused.
    let (status, _, _) = client
        .call(
            "PUT",
            &format!("/v1/content/{id}/destinations/{dest}"),
            Some(json!({"when": "now"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, body, _) = client
        .call(
            "PUT",
            &format!("/v1/destinations/{dest}/github"),
            Some(json!({
                "owner": "acme", "name": "site", "baseBranch": "main",
                "contentRoot": "content/blog",
                "publicRoutePattern": "/blog/{slug}",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["repository"], "acme/site");

    let (status, accepted, _) = client
        .call(
            "PUT",
            &format!("/v1/content/{id}/destinations/{dest}"),
            Some(json!({"when": "now"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let delivery_id: Uuid = accepted["deliveryId"].as_str().unwrap().parse().unwrap();

    // The worker: open the pull request, then poll it.
    assert!(matches!(
        delivery::run(&pool, Some(&github), delivery_id, false).await,
        Ok(Step::RetryAt(_))
    ));
    {
        let hub = hub.lock().unwrap();
        assert_eq!(hub.files.len(), 1);
        assert_eq!(hub.files[0].0, "content/blog/how-geo-works.md");
        assert!(hub.files[0].1.contains("title: \"How GEO works\""));
    }
    let (_, deliveries, _) = client
        .call("GET", &format!("/v1/content/{id}/deliveries"), None)
        .await;
    assert_eq!(deliveries["deliveries"][0]["state"], "in_review");
    assert_eq!(
        deliveries["deliveries"][0]["prUrl"],
        "https://github.com/acme/site/pull/3"
    );

    // While it is open, a second publish conflicts.
    let (status, _, _) = client
        .call(
            "PUT",
            &format!("/v1/content/{id}/destinations/{dest}"),
            Some(json!({"when": "now"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    assert!(matches!(
        delivery::run(&pool, Some(&github), delivery_id, false).await,
        Ok(Step::RetryAt(_))
    ));
    hub.lock().unwrap().merged = true;
    assert_eq!(
        delivery::run(&pool, Some(&github), delivery_id, false).await,
        Ok(Step::Done)
    );

    let (_, deliveries, _) = client
        .call("GET", &format!("/v1/content/{id}/deliveries"), None)
        .await;
    assert_eq!(deliveries["deliveries"][0]["state"], "live");
    let (_, publications, _) = client
        .call("GET", &format!("/v1/content/{id}/publications"), None)
        .await;
    assert_eq!(
        publications["publications"][0]["url"],
        "https://acme.com/blog/how-geo-works"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn withdrawing_closes_the_pull_request(pool: PgPool) {
    let hub = Shared::default();
    let url = fake_github(hub.clone()).await;
    let github = Arc::new(App::new(
        "1".into(),
        EncodingKey::from_secret(b"test"),
        Algorithm::HS256,
        &url,
    ));
    let (client, project) = setup(pool.clone(), Some(github.clone())).await;
    let opp = opportunity(&pool, project).await;
    let (_, created, _) = client
        .call(
            "POST",
            "/v1/content",
            Some(
                json!({"opportunityId": opp, "article": article("how-geo-works", "## A\n\ntext")}),
            ),
        )
        .await;
    let id = created["contentId"].as_str().unwrap().to_owned();
    let (_, dest, _) = client
        .call(
            "POST",
            "/v1/destinations",
            Some(json!({"platform": "github"})),
        )
        .await;
    let dest = dest["destinationId"].as_str().unwrap().to_owned();
    client
        .call(
            "PUT",
            &format!("/v1/destinations/{dest}/github"),
            Some(json!({
                "owner": "acme", "name": "site", "baseBranch": "main",
                "contentRoot": "", "publicRoutePattern": "/{slug}",
            })),
        )
        .await;
    let (_, accepted, _) = client
        .call(
            "PUT",
            &format!("/v1/content/{id}/destinations/{dest}"),
            Some(json!({"when": "now"})),
        )
        .await;
    let delivery_id: Uuid = accepted["deliveryId"].as_str().unwrap().parse().unwrap();
    delivery::run(&pool, Some(&github), delivery_id, false)
        .await
        .unwrap();

    let (status, _, _) = client
        .call(
            "DELETE",
            &format!("/v1/content/{id}/destinations/{dest}"),
            None,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an open pull request needs withdraw"
    );
    let (status, _, _) = client
        .call(
            "DELETE",
            &format!("/v1/content/{id}/destinations/{dest}?withdraw=true"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    delivery::run(&pool, Some(&github), delivery_id, true)
        .await
        .unwrap();
    assert!(hub.lock().unwrap().closed);
    let status: String = sqlx::query_scalar("SELECT status FROM deliveries WHERE id = $1")
        .bind(delivery_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "cancelled");
}
