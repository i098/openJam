//! Drive the runner over HTTP with a fake `claude` script. No test calls Anthropic.

use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

use serde_json::{Value, json};

const SECRET: &str = "s3cret-test";

/// One fake for all tests, made once. Writing an executable while other threads spawn
/// processes can fail with "text file busy", so no test writes its own.
const FAKE: &str = r#"#!/bin/sh
if [ "$1" = auth ]; then
  echo "Login method: Claude Max account"
  echo "Sign-in: test@example.com"
  exit 0
fi
prompt=$(cat)
case "$prompt" in
  success|slow)
    [ "$prompt" = slow ] && sleep 1
    env > "$CLAUDE_CONFIG_DIR/env.txt"
    echo "$@" > "$CLAUDE_CONFIG_DIR/args.txt"
    mkdir -p "$CLAUDE_CONFIG_DIR/projects/p"
    echo '{}' > "$CLAUDE_CONFIG_DIR/projects/p/session.jsonl"
    cat <<'EOF'
{"type":"system","subtype":"init","session_id":"s1","tools":["WebSearch"],"model":"claude-sonnet-4-5"}
{"type":"assistant","message":{"model":"claude-sonnet-4-5","content":[{"type":"tool_use","id":"tu1","name":"WebSearch","input":{"query":"best crm"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tu1","content":"Web search results for query: \"best crm\"\n\nLinks: [{\"title\":\"CRM One\",\"url\":\"https://one.example/crm\"},{\"title\":\"CRM Two\",\"url\":\"https://two.example/list\"}]"}]}}
{"type":"result","subtype":"success","is_error":false,"result":"Try CRM One.","usage":{"input_tokens":12,"output_tokens":34},"modelUsage":{"claude-sonnet-4-5":{}}}
EOF
    ;;
  limit)
    echo '{"type":"result","subtype":"success","is_error":false,"result":"Claude AI usage limit reached|1767225600"}'
    ;;
  hang)
    echo $$ > "$CLAUDE_CONFIG_DIR/pgid"
    sleep 30
    ;;
  fail)
    echo "boom" >&2
    exit 3
    ;;
esac
"#;

static FAKE_BIN: LazyLock<(tempfile::TempDir, PathBuf)> = LazyLock::new(|| {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claude");
    std::fs::write(&path, FAKE).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, path)
});

struct Runner {
    url: String,
    logins: tempfile::TempDir,
    http: reqwest::Client,
}

impl Runner {
    async fn start() -> Runner {
        let logins = tempfile::tempdir().unwrap();
        std::fs::create_dir(logins.path().join("acct")).unwrap();
        let cfg = openjam_runner::Config {
            secret: SECRET.into(),
            claude_bin: FAKE_BIN.1.to_string_lossy().into_owned(),
            logins_dir: logins.path().to_path_buf(),
            max_run: Duration::from_secs(120),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, openjam_runner::router(cfg)).await });
        Runner {
            url,
            logins,
            http: reqwest::Client::new(),
        }
    }

    async fn run(&self, folder: &str, prompt: &str, timeout_ms: u64) -> (u16, Value) {
        let body =
            json!({"run_id": "r1", "folder": folder, "prompt": prompt, "timeout_ms": timeout_ms});
        let resp = self
            .http
            .post(format!("{}/v1/run", self.url))
            .bearer_auth(SECRET)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.logins.path().join("acct").join(name)).unwrap()
    }
}

#[tokio::test]
async fn success_returns_text_sources_and_clean_env() {
    let r = Runner::start().await;
    let (status, body) = r.run("acct", "success", 10_000).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["text"], "Try CRM One.");
    assert_eq!(body["search_used"], true);
    assert_eq!(
        body["sources"],
        json!([
            {"url": "https://one.example/crm", "title": "CRM One"},
            {"url": "https://two.example/list", "title": "CRM Two"},
        ])
    );
    assert_eq!(body["usage"], json!({"tokens_in": 12, "tokens_out": 34}));
    assert_eq!(body["model"], "claude-sonnet-4-5");
    assert_eq!(body["limit"], Value::Null);
    assert_eq!(body["logged_out"], false);

    let env = r.file("env.txt");
    assert!(!env.contains("RUNNER_SHARED_SECRET"), "{env}");
    assert!(!env.contains(SECRET), "{env}");
    assert!(env.contains("DISABLE_AUTOUPDATER=1"), "{env}");
    assert!(
        env.contains("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1"),
        "{env}"
    );
    let config_dir = r.logins.path().join("acct");
    assert!(env.contains(&format!("CLAUDE_CONFIG_DIR={}", config_dir.display())));
    assert_eq!(
        r.file("args.txt").trim(),
        "-p --output-format stream-json --verbose --tools WebSearch --max-turns 4"
    );
    assert!(!config_dir.join("projects/p/session.jsonl").exists());
}

#[tokio::test]
async fn auth_and_folder_checks() {
    let r = Runner::start().await;
    let resp = r
        .http
        .post(format!("{}/v1/run", r.url))
        .bearer_auth("wrong")
        .json(&json!({"run_id": "r", "folder": "acct", "prompt": "success", "timeout_ms": 1000}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 401);
    let resp = r
        .http
        .get(format!("{}/v1/claude/accounts", r.url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 401);
    let health = r
        .http
        .get(format!("{}/healthz", r.url))
        .send()
        .await
        .unwrap();
    assert_eq!(health.text().await.unwrap(), "ok");

    assert_eq!(r.run("../acct", "success", 1000).await.0, 400);
    assert_eq!(r.run("a/b", "success", 1000).await.0, 400);
    assert_eq!(r.run(".hidden", "success", 1000).await.0, 400);
    assert_eq!(r.run("missing", "success", 1000).await.0, 404);

    let (status, body) = r.run("acct", "fail", 5000).await;
    assert_eq!(status, 502);
    assert_eq!(body["code"], "cli_failed");
    assert!(body["message"].as_str().unwrap().contains("boom"));
}

#[tokio::test]
async fn second_run_on_a_busy_folder_gets_503() {
    let r = Runner::start().await;
    let (a, b) = tokio::join!(r.run("acct", "slow", 10_000), r.run("acct", "slow", 10_000));
    let mut codes = [a.0, b.0];
    codes.sort();
    assert_eq!(codes, [200, 503]);
    let busy = if a.0 == 503 { a.1 } else { b.1 };
    assert_eq!(busy["code"], "runner_busy");
    // The folder is free again after the run.
    assert_eq!(r.run("acct", "success", 10_000).await.0, 200);
}

#[tokio::test]
async fn usage_limit_sets_limit_and_empty_text() {
    let r = Runner::start().await;
    let (status, body) = r.run("acct", "limit", 10_000).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["text"], "");
    assert_eq!(body["limit"], json!({"reset_at": "2026-01-01T00:00:00Z"}));
}

#[tokio::test]
async fn timeout_kills_the_process_group() {
    let r = Runner::start().await;
    let (status, body) = r.run("acct", "hang", 500).await;
    assert_eq!(status, 504);
    assert_eq!(body["code"], "timeout");
    let pgid: u32 = r.file("pgid").trim().parse().unwrap();
    // The killed `sleep` is reaped by init a little later; zombies do not count.
    for _ in 0..50 {
        if live_in_group(pgid) == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("process group {pgid} is still alive");
}

/// Count processes in a group that are not zombies, from `/proc/<pid>/stat`.
fn live_in_group(pgid: u32) -> usize {
    std::fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("stat")).ok())
        .filter(|stat| {
            // Fields after the `(comm)`: state, ppid, pgrp.
            let rest = stat.rsplit_once(')').map_or("", |(_, r)| r);
            let fields: Vec<&str> = rest.split_whitespace().collect();
            fields.len() > 2 && fields[0] != "Z" && fields[2] == pgid.to_string()
        })
        .count()
}

#[tokio::test]
async fn accounts_list_and_delete() {
    let r = Runner::start().await;
    std::fs::create_dir(r.logins.path().join("work")).unwrap();
    std::fs::write(r.logins.path().join("work/.openjam-label"), "Work Max\n").unwrap();
    let list: Value = r
        .http
        .get(format!("{}/v1/claude/accounts", r.url))
        .bearer_auth(SECRET)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        list,
        json!([
            {"folder": "acct", "label": "acct", "sign_in_line": "Sign-in: test@example.com"},
            {"folder": "work", "label": "Work Max", "sign_in_line": "Sign-in: test@example.com"},
        ])
    );
    let resp = r
        .http
        .delete(format!("{}/v1/claude/accounts/work", r.url))
        .bearer_auth(SECRET)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 204);
    assert!(!r.logins.path().join("work").exists());
}
