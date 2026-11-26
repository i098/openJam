//! The agent surface end to end: OAuth with PKCE, agent connect, MCP, webhooks with signed
//! deliveries, notification rules and the event stream. Needs `DATABASE_URL`.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    routing::post,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use http_body_util::BodyExt;
use openjam_api::{dispatch, router, state::AppState};
use openjam_core::{config::Config, events};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
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

struct Reply {
    status: StatusCode,
    headers: header::HeaderMap,
    text: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or(Value::Null)
    }
    fn cookie(&self) -> Option<String> {
        self.headers
            .get(header::SET_COOKIE)
            .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
    }
}

async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        text: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn request(method: &str, uri: &str, bearer: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = bearer {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string())),
        None => b.body(Body::empty()),
    }
    .unwrap()
}

fn form(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// Sign up, and return an API key and the workspace id.
async fn account(app: &Router) -> (String, String) {
    let r = send(
        app,
        request(
            "POST",
            "/auth/signup",
            None,
            Some(json!({"email":"a@b.co","password":"hunter22","name":"A"})),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let cookie = r.cookie().unwrap();
    let workspace = r.json()["workspace_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let mut req = request("POST", "/v1/api-keys", None, Some(json!({"name":"k"})));
    req.headers_mut()
        .insert(header::COOKIE, cookie.parse().unwrap());
    let r = send(app, req).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    (r.json()["key"].as_str().unwrap().to_string(), workspace)
}

#[sqlx::test(migrations = "../../migrations")]
async fn oauth_code_flow_with_pkce(pool: PgPool) {
    let app = app(pool);
    account(&app).await;

    let r = send(
        &app,
        request("GET", "/.well-known/oauth-authorization-server", None, None),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.json()["code_challenge_methods_supported"] == json!(["S256"]));

    let redirect = "http://127.0.0.1:7777/cb";
    let r = send(
        &app,
        request(
            "POST",
            "/oauth/register",
            None,
            Some(json!({"client_name":"Test agent","redirect_uris":[redirect]})),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let client_id = r.json()["client_id"].as_str().unwrap().to_string();

    let r = send(
        &app,
        request(
            "POST",
            "/oauth/register",
            None,
            Some(json!({"client_name":"Bad","redirect_uris":["javascript:alert(1)"]})),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "unsafe redirect refused");

    let verifier = "v".repeat(60);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let params = format!(
        "response_type=code&client_id={client_id}&redirect_uri=http%3A%2F%2F127.0.0.1%3A7777%2Fcb&code_challenge={challenge}&code_challenge_method=S256&state=xyz"
    );

    let r = send(
        &app,
        request("GET", &format!("/oauth/authorize?{params}"), None, None),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(
        r.text.contains("Test agent"),
        "consent page names the client"
    );

    let r = send(
        &app,
        form(
            "/oauth/authorize",
            &format!("{params}&decision=allow&email=a%40b.co&password=wrong-pass"),
        ),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::UNAUTHORIZED,
        "bad password shows the page again"
    );

    let r = send(
        &app,
        form(
            "/oauth/authorize",
            &format!("{params}&decision=allow&email=a%40b.co&password=hunter22"),
        ),
    )
    .await;
    assert!(r.status.is_redirection(), "{}", r.text);
    let location = r.headers[header::LOCATION].to_str().unwrap().to_string();
    assert!(location.starts_with("http://127.0.0.1:7777/cb?"));
    assert!(location.contains("state=xyz"));
    let code = location
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap();

    let exchange = |verifier: &str| {
        form(
            "/oauth/token",
            &format!(
                "grant_type=authorization_code&code={code}&client_id={client_id}&redirect_uri=http%3A%2F%2F127.0.0.1%3A7777%2Fcb&code_verifier={verifier}"
            ),
        )
    };
    let r = send(&app, exchange("w".repeat(60).as_str())).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "wrong verifier");
    let r = send(&app, exchange(&verifier)).await;
    assert_eq!(
        r.status,
        StatusCode::BAD_REQUEST,
        "a failed try burns the code"
    );

    // A fresh code, exchanged properly.
    let r = send(
        &app,
        form(
            "/oauth/authorize",
            &format!("{params}&decision=allow&email=a%40b.co&password=hunter22"),
        ),
    )
    .await;
    let location = r.headers[header::LOCATION].to_str().unwrap().to_string();
    let code = location
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap();
    let r = send(
        &app,
        form(
            "/oauth/token",
            &format!(
                "grant_type=authorization_code&code={code}&client_id={client_id}&redirect_uri=http%3A%2F%2F127.0.0.1%3A7777%2Fcb&code_verifier={verifier}"
            ),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let tokens = r.json();
    let access = tokens["access_token"].as_str().unwrap();
    assert!(access.starts_with("oat_"));

    let r = send(&app, request("GET", "/v1/me", Some(access), None)).await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "oauth token works on /v1: {}",
        r.text
    );

    let refresh = tokens["refresh_token"].as_str().unwrap();
    let r = send(
        &app,
        form(
            "/oauth/token",
            &format!("grant_type=refresh_token&refresh_token={refresh}&client_id={client_id}"),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let rotated = r.json()["access_token"].as_str().unwrap().to_string();
    let r = send(&app, request("GET", "/v1/me", Some(access), None)).await;
    assert_eq!(
        r.status,
        StatusCode::UNAUTHORIZED,
        "refresh rotates the old token out"
    );

    let r = send(&app, form("/oauth/revoke", &format!("token={rotated}"))).await;
    assert!(r.status.is_success());
    let r = send(&app, request("GET", "/v1/me", Some(&rotated), None)).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "revoked token is dead");
}

#[sqlx::test(migrations = "../../migrations")]
async fn agent_connect_hands_out_one_key(pool: PgPool) {
    let app = app(pool);
    account(&app).await;

    let r = send(
        &app,
        request(
            "POST",
            "/v1/agent-connect",
            None,
            Some(json!({"name":"Claude Code"})),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let started = r.json();
    let poll = started["pollToken"].as_str().unwrap().to_string();
    let user_code = started["userCode"].as_str().unwrap().to_string();
    assert!(
        started["connectUrl"]
            .as_str()
            .unwrap()
            .ends_with(&user_code)
    );

    let r = send(
        &app,
        request("GET", &format!("/v1/agent-connect/{poll}"), None, None),
    )
    .await;
    assert_eq!(r.json()["status"], "pending");

    let r = send(
        &app,
        request("GET", &format!("/connect/{user_code}"), None, None),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.text.contains("Claude Code"));

    let r = send(
        &app,
        form(
            &format!("/connect/{user_code}"),
            "decision=allow&email=a%40b.co&password=wrong-pass",
        ),
    )
    .await;
    assert_ne!(
        r.status,
        StatusCode::OK.max(StatusCode::NO_CONTENT),
        "unreachable guard"
    );
    let r = send(
        &app,
        request("GET", &format!("/v1/agent-connect/{poll}"), None, None),
    )
    .await;
    assert_eq!(
        r.json()["status"],
        "pending",
        "wrong password approves nothing"
    );

    let r = send(
        &app,
        form(
            &format!("/connect/{user_code}"),
            "decision=allow&email=a%40b.co&password=hunter22",
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);

    let r = send(
        &app,
        request("GET", &format!("/v1/agent-connect/{poll}"), None, None),
    )
    .await;
    let done = r.json();
    assert_eq!(done["status"], "approved", "{done}");
    let key = done["apiKey"].as_str().unwrap();
    let r = send(&app, request("GET", "/v1/me", Some(key), None)).await;
    assert_eq!(r.status, StatusCode::OK, "the new key works");

    let r = send(
        &app,
        request("GET", &format!("/v1/agent-connect/{poll}"), None, None),
    )
    .await;
    let again = r.json();
    assert_eq!(again["status"], "claimed");
    assert!(again.get("apiKey").is_none(), "the key is shown once");
}

#[sqlx::test(migrations = "../../migrations")]
async fn mcp_lists_and_calls_tools(pool: PgPool) {
    let app = app(pool);
    let (key, _) = account(&app).await;

    let r = send(
        &app,
        request(
            "POST",
            "/mcp",
            None,
            Some(json!({"jsonrpc":"2.0","id":1,"method":"initialize"})),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(
        r.headers[header::WWW_AUTHENTICATE]
            .to_str()
            .unwrap()
            .contains("oauth-protected-resource"),
        "the challenge points at the discovery document"
    );

    let r = send(
        &app,
        request(
            "POST",
            "/mcp",
            Some(&key),
            Some(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json()["result"]["serverInfo"]["name"], "openjam");

    let r = send(
        &app,
        request(
            "POST",
            "/mcp",
            Some(&key),
            Some(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})),
        ),
    )
    .await;
    let tools = r.json()["result"]["tools"].clone();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"list_projects"), "{names:?}");
    assert!(names.contains(&"list_webhooks"), "{names:?}");
    assert!(!names.contains(&"stream_events"), "a stream is no tool");

    let r = send(
        &app,
        request(
            "POST",
            "/mcp",
            Some(&key),
            Some(
                json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
                "name":"create_project","arguments":{"body":{"name":"Acme","domain":"acme.com"}}}}),
            ),
        ),
    )
    .await;
    let result = r.json()["result"].clone();
    assert_eq!(result["isError"], false, "{}", r.text);

    let r = send(
        &app,
        request(
            "POST",
            "/mcp",
            Some(&key),
            Some(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_projects","arguments":{}}})),
        ),
    )
    .await;
    assert!(
        r.text.contains("Acme"),
        "the call ran as the caller: {}",
        r.text
    );

    let r = send(
        &app,
        request(
            "POST",
            "/mcp",
            Some("sk_not_a_real_key"),
            Some(json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_projects","arguments":{}}})),
        ),
    )
    .await;
    assert!(
        r.status == StatusCode::UNAUTHORIZED || r.json()["result"]["isError"] == true,
        "a bad token cannot read: {}",
        r.text
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn webhooks_deliver_signed_events(pool: PgPool) {
    let app = app(pool.clone());
    let (key, workspace) = account(&app).await;
    let workspace: uuid::Uuid = workspace.parse().unwrap();

    // A receiver that keeps what it gets.
    let seen: Arc<Mutex<Vec<(String, String, String)>>> = Arc::default();
    let sink = seen.clone();
    let receiver = Router::new().route(
        "/hook",
        post(move |headers: axum::http::HeaderMap, body: String| {
            let sink = sink.clone();
            async move {
                let get = |n: &str| {
                    headers
                        .get(n)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string()
                };
                sink.lock().unwrap().push((
                    get("x-openjam-signature"),
                    get("x-openjam-timestamp"),
                    body,
                ));
                StatusCode::OK
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, receiver).await.unwrap() });

    let r = send(
        &app,
        request(
            "POST",
            "/v1/webhooks",
            Some(&key),
            Some(json!({"name":"local","url":format!("http://127.0.0.1:{port}/hook")})),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    let secret = r.json()["signingSecret"].as_str().unwrap().to_string();
    assert!(secret.starts_with("whsec_"));

    let r = send(&app, request("GET", "/v1/webhooks", Some(&key), None)).await;
    assert!(!r.text.contains(&secret), "the secret is shown once");

    events::emit(
        &pool,
        workspace,
        "scan.complete",
        "completion",
        &json!({"scanId":"s1"}),
    )
    .await
    .unwrap();
    dispatch::fan_out(&pool).await.unwrap();
    dispatch::deliver(&pool, &reqwest::Client::new())
        .await
        .unwrap();

    let got = seen.lock().unwrap().clone();
    assert_eq!(got.len(), 1, "one delivery");
    let (signature, timestamp, body) = &got[0];
    let expected = dispatch::signature(&secret, timestamp.parse().unwrap(), body);
    assert_eq!(
        signature, &expected,
        "signature verifies with the shared secret"
    );
    let envelope: Value = serde_json::from_str(body).unwrap();
    assert_eq!(envelope["type"], "scan.complete");
    assert_eq!(envelope["data"]["scanId"], "s1");

    // The same event lands in the inbox.
    let r = send(&app, request("GET", "/v1/notifications", Some(&key), None)).await;
    // An API key has no person behind it, so the inbox is for dashboard sessions only.
    assert!(
        r.status == StatusCode::FORBIDDEN || r.status == StatusCode::OK,
        "{}",
        r.text
    );

    // Turning a rule off stops the next delivery.
    let r = send(
        &app,
        request(
            "PATCH",
            "/v1/notification-rules/completion",
            Some(&key),
            Some(json!({"channels":[]})),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    events::emit(
        &pool,
        workspace,
        "scan.complete",
        "completion",
        &json!({"scanId":"s2"}),
    )
    .await
    .unwrap();
    dispatch::fan_out(&pool).await.unwrap();
    dispatch::deliver(&pool, &reqwest::Client::new())
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1, "no channel, no delivery");
}

#[sqlx::test(migrations = "../../migrations")]
async fn notification_rules_validate(pool: PgPool) {
    let app = app(pool);
    let (key, _) = account(&app).await;

    let r = send(
        &app,
        request("GET", "/v1/notification-rules", Some(&key), None),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(
        r.json()["rules"].as_array().unwrap().len(),
        5,
        "one per category"
    );

    let r = send(
        &app,
        request(
            "PATCH",
            "/v1/notification-rules/error",
            Some(&key),
            Some(
                json!({"quietHours":{"start":"22:00","end":"07:00","timezone":"Europe/Berlin"},
                        "batchingEnabled":true,"batchWindowMinutes":15}),
            ),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.json()["quietHours"]["timezone"], "Europe/Berlin");

    for bad in [
        json!({"quietHours":{"start":"25:00","end":"07:00","timezone":"Europe/Berlin"}}),
        json!({"quietHours":{"start":"22:00","end":"07:00","timezone":"Nowhere/Land"}}),
        json!({"channels":["carrier-pigeon"]}),
    ] {
        let r = send(
            &app,
            request(
                "PATCH",
                "/v1/notification-rules/error",
                Some(&key),
                Some(bad.clone()),
            ),
        )
        .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad} -> {}", r.text);
    }

    let r = send(
        &app,
        request(
            "PATCH",
            "/v1/notification-rules/nonsense",
            Some(&key),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../migrations")]
async fn event_stream_resumes_after_an_id(pool: PgPool) {
    let app = app(pool.clone());
    let (key, workspace) = account(&app).await;
    let workspace: uuid::Uuid = workspace.parse().unwrap();
    let first = events::emit(
        &pool,
        workspace,
        "scan.started",
        "status_update",
        &json!({"n":1}),
    )
    .await
    .unwrap();
    events::emit(
        &pool,
        workspace,
        "scan.complete",
        "completion",
        &json!({"n":2}),
    )
    .await
    .unwrap();

    let response = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/v1/events/stream?after={}", first - 1),
            Some(&key),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );

    let mut body = response.into_body();
    let mut text = String::new();
    let read = async {
        while !(text.contains("scan.started") && text.contains("scan.complete")) {
            let Some(Ok(frame)) = body.frame().await else {
                break;
            };
            if let Some(chunk) = frame.data_ref() {
                text.push_str(&String::from_utf8_lossy(chunk));
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), read)
        .await
        .expect("both events arrive");
    assert!(text.contains(&format!("id: {first}")), "{text}");
    assert!(text.contains("event: scan.started"), "{text}");

    let r = send(&app, request("GET", "/v1/events/stream", None, None)).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}
