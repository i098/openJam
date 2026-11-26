//! End-to-end HTTP flow: sign up, make a key, use the key. Needs `DATABASE_URL`.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use openjam_api::{router, state::AppState};
use openjam_core::config::Config;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

fn app(pool: PgPool) -> Router {
    let config = Config {
        database_url: "unused in tests".into(),
        bind: "127.0.0.1:0".into(),
        allow_signup: true,
        cookie_secure: false,
        session_days: 30,
        scan_cap_per_day: None,
    };
    router(AppState {
        pool,
        config: Arc::new(config),
    })
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Option<String>, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        json!(null)
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, cookie, body)
}

fn json_request(method: &str, uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn signup_key_and_projects(pool: PgPool) {
    let app = app(pool);

    let (status, cookie, body) = send(
        &app,
        json_request(
            "POST",
            "/auth/signup",
            json!({"email":"a@b.co","password":"hunter22","name":"A"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "signup: {body}");
    assert_eq!(body["user"]["name"], "A", "signup echoes the name: {body}");
    let cookie = cookie.expect("session cookie");
    assert!(cookie.starts_with("openjam_session="));

    let (status, _, body) = send(
        &app,
        json_request("POST", "/v1/api-keys", json!({"name":"laptop"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "no session: {body}");

    let mut key_request = json_request("POST", "/v1/api-keys", json!({"name":"laptop"}));
    key_request
        .headers_mut()
        .insert(header::COOKIE, cookie.parse().unwrap());
    let (status, _, body) = send(&app, key_request).await;
    assert_eq!(status, StatusCode::CREATED, "create key: {body}");
    let key = body["key"].as_str().unwrap().to_string();
    assert!(key.starts_with("sk_"));

    let mut create = json_request(
        "POST",
        "/v1/projects",
        json!({"name":"Acme","domain":"https://acme.com/"}),
    );
    create.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {key}").parse().unwrap(),
    );
    let (status, _, body) = send(&app, create).await;
    assert_eq!(status, StatusCode::CREATED, "create project: {body}");
    assert_eq!(body["domain"], "acme.com");
    assert_eq!(body["siteUrl"], "https://acme.com");

    let list = Request::builder()
        .uri("/v1/projects")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(&app, list).await;
    assert_eq!(status, StatusCode::OK, "list: {body}");
    assert_eq!(body["projects"].as_array().unwrap().len(), 1);
    assert_eq!(body["projects"][0]["name"], "Acme");
}

#[sqlx::test(migrations = "../../migrations")]
async fn rejects_bad_requests(pool: PgPool) {
    let app = app(pool);

    let (status, _, body) = send(
        &app,
        Request::builder()
            .uri("/v1/projects")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "unauthorized");

    let (status, _, body) = send(
        &app,
        json_request(
            "POST",
            "/auth/signup",
            json!({"email":"x@y.co","password":"short"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_request");

    let (status, _, _body) = send(
        &app,
        Request::builder()
            .uri("/v1/analytics/overview")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "stubs still check credentials"
    );

    let (status, cookie, _) = send(
        &app,
        json_request(
            "POST",
            "/auth/signup",
            json!({"email":"q@r.co","password":"hunter22"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let mut stub = Request::builder()
        .uri("/v1/analytics/overview")
        .body(Body::empty())
        .unwrap();
    stub.headers_mut()
        .insert(header::COOKIE, cookie.unwrap().parse().unwrap());
    let (status, _, body) = send(&app, stub).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(body["code"], "not_implemented");

    let (status, _, _body) = send(
        &app,
        Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrations = "../../migrations")]
async fn readyz_and_metrics(pool: PgPool) {
    let app = app(pool);
    let get = |uri: &str| Request::builder().uri(uri).body(Body::empty()).unwrap();

    let (status, _, body) = send(&app, get("/readyz")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["database"], "up");

    let response = app.clone().oneshot(get("/metrics")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("openjam_db_up 1"));
    assert!(text.contains("openjam_jobs{status=\"queued\"} 0"));
    assert!(text.contains("openjam_http_requests_total{class=\"2xx\"}"));
}
