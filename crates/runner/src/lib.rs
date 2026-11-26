//! The openJam runner: a small HTTP server that runs the Claude Code CLI for scans.

mod stream;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use axum::extract::{DefaultBodyLimit, Path as UrlPath, Request, State};
use axum::http::{StatusCode, header::AUTHORIZATION};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use serde_json::json;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use stream::{Failure, Stream};

/// Name prefix of the run temp dirs, so a start can sweep dirs that a crash left.
pub const TMP_PREFIX: &str = "openjam-run-";
const BODY_LIMIT: usize = 64 * 1024;
const AUTH_STATUS_TIMEOUT: Duration = Duration::from_secs(10);
/// File mtimes can be a little older than the clock read before the spawn.
const MTIME_SLACK: Duration = Duration::from_secs(1);

pub struct Config {
    pub secret: String,
    pub claude_bin: String,
    pub logins_dir: PathBuf,
    pub max_run: Duration,
}

impl Config {
    /// Read the bind address and the config from the `RUNNER_*` env vars.
    pub fn from_env() -> Result<(String, Config), String> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let secret = match (
            var("RUNNER_SHARED_SECRET"),
            var("RUNNER_SHARED_SECRET_FILE"),
        ) {
            (Some(secret), _) => secret,
            (None, Some(file)) => std::fs::read_to_string(&file)
                .map_err(|e| format!("cannot read RUNNER_SHARED_SECRET_FILE {file}: {e}"))?
                .trim()
                .to_string(),
            (None, None) => return Err("RUNNER_SHARED_SECRET is required".into()),
        };
        if secret.is_empty() {
            return Err("the shared secret is empty".into());
        }
        let max_run = match var("RUNNER_MAX_RUN_SECS") {
            Some(v) => v
                .parse()
                .map_err(|_| format!("RUNNER_MAX_RUN_SECS is not a number: {v}"))?,
            None => 120,
        };
        Ok((
            var("RUNNER_BIND").unwrap_or_else(|| "0.0.0.0:8080".into()),
            Config {
                secret,
                claude_bin: var("RUNNER_CLAUDE_BIN").unwrap_or_else(|| "claude".into()),
                logins_dir: var("RUNNER_LOGINS_DIR")
                    .unwrap_or_else(|| "/claude-logins".into())
                    .into(),
                max_run: Duration::from_secs(max_run),
            },
        ))
    }
}

#[derive(Clone)]
struct AppState {
    cfg: Arc<Config>,
    busy: Arc<Mutex<HashSet<String>>>,
}

/// Holds a folder as busy until it is dropped.
struct Busy {
    busy: Arc<Mutex<HashSet<String>>>,
    folder: String,
}

impl Drop for Busy {
    fn drop(&mut self) {
        lock(&self.busy).remove(&self.folder);
    }
}

fn lock(busy: &Mutex<HashSet<String>>) -> std::sync::MutexGuard<'_, HashSet<String>> {
    busy.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Kills the whole process group of a child when dropped.
struct Group(Option<u32>);

impl Drop for Group {
    fn drop(&mut self) {
        if let Some(pid) = self.0.and_then(|p| i32::try_from(p).ok()) {
            let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
        }
    }
}

impl AppState {
    fn take(&self, folder: &str) -> Option<Busy> {
        lock(&self.busy).insert(folder.to_string()).then(|| Busy {
            busy: self.busy.clone(),
            folder: folder.to_string(),
        })
    }

    /// The login dir for a folder name, or the error status and code.
    fn folder_dir(&self, folder: &str) -> Result<PathBuf, (StatusCode, &'static str)> {
        if !valid_folder(folder) {
            return Err((StatusCode::BAD_REQUEST, "bad_folder"));
        }
        let dir = self.cfg.logins_dir.join(folder);
        if !dir.is_dir() {
            return Err((StatusCode::NOT_FOUND, "folder_not_found"));
        }
        Ok(dir)
    }
}

/// A plain dir name: no path separator, no `..`, not hidden.
fn valid_folder(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', '\0'])
}

fn error(status: StatusCode, code: &str, message: Option<String>) -> Response {
    (status, Json(json!({ "code": code, "message": message }))).into_response()
}

pub fn router(cfg: Config) -> Router {
    let state = AppState {
        cfg: Arc::new(cfg),
        busy: Arc::default(),
    };
    Router::new()
        .route("/v1/run", post(run))
        .route("/v1/claude/accounts", get(accounts))
        .route("/v1/claude/accounts/{folder}", delete(delete_account))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth))
        .route("/healthz", get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(state)
}

async fn auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|token| bool::from(token.as_bytes().ct_eq(state.cfg.secret.as_bytes())));
    if ok {
        next.run(req).await
    } else {
        error(StatusCode::UNAUTHORIZED, "unauthorized", None)
    }
}

/// A command for the CLI with a cleared env, its own process group and `home` as cwd.
fn claude_command(cfg: &Config, config_dir: &Path, home: &Path) -> Command {
    let mut cmd = Command::new(&cfg.claude_bin);
    cmd.env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("CLAUDE_CONFIG_DIR", config_dir)
        .env("DISABLE_AUTOUPDATER", "1")
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .current_dir(home)
        .process_group(0)
        .kill_on_drop(true);
    cmd
}

fn temp_dir() -> std::io::Result<tempfile::TempDir> {
    tempfile::Builder::new().prefix(TMP_PREFIX).tempdir()
}

/// Remove run temp dirs that an earlier process left behind.
pub fn sweep_temp_dirs() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(TMP_PREFIX) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[derive(Deserialize)]
struct RunRequest {
    run_id: String,
    folder: String,
    prompt: String,
    model: Option<String>,
    timeout_ms: u64,
}

async fn run(State(state): State<AppState>, Json(req): Json<RunRequest>) -> Response {
    let dir = match state.folder_dir(&req.folder) {
        Ok(dir) => dir,
        Err((status, code)) => return error(status, code, None),
    };
    let Some(_busy) = state.take(&req.folder) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "runner_busy", None);
    };
    let started = Instant::now();
    let since = SystemTime::now() - MTIME_SLACK;
    let reply = execute(&state.cfg, &dir, &req).await;
    clean_sessions(&dir.join("projects"), since);
    tracing::info!(
        run_id = %req.run_id,
        folder = %req.folder,
        status = reply.status().as_u16(),
        ms = started.elapsed().as_millis(),
        "run done"
    );
    reply
}

async fn execute(cfg: &Config, dir: &Path, req: &RunRequest) -> Response {
    let cli_failed = |message: String| error(StatusCode::BAD_GATEWAY, "cli_failed", Some(message));
    let tmp = match temp_dir() {
        Ok(tmp) => tmp,
        Err(e) => return cli_failed(format!("cannot make a temp dir: {e}")),
    };
    let mut cmd = claude_command(cfg, dir, tmp.path());
    cmd.args(["-p", "--output-format", "stream-json", "--verbose"])
        .args(["--tools", "WebSearch", "--max-turns", "4"]);
    if let Some(model) = &req.model {
        cmd.args(["--model", model]);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return cli_failed(format!("cannot start {}: {e}", cfg.claude_bin)),
    };
    let group = Group(child.id());
    let (Some(mut stdin), Some(stdout), Some(mut stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return cli_failed("no child pipes".into());
    };

    let prompt = req.prompt.as_bytes();
    let io = async {
        let write = async move {
            // The CLI can exit before it reads all of the prompt; the exit code tells.
            let _ = stdin.write_all(prompt).await;
        };
        let read = async {
            let mut parsed = Stream::default();
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                parsed.line(&line);
            }
            parsed
        };
        let read_err = async {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf).await;
            buf
        };
        let ((), parsed, err) = tokio::join!(write, read, read_err);
        (parsed, err, child.wait().await)
    };
    let limit = Duration::from_millis(req.timeout_ms).min(cfg.max_run);
    let outcome = tokio::time::timeout(limit, io).await;
    drop(group);
    let Ok((parsed, err, status)) = outcome else {
        let _ = child.wait().await;
        return error(StatusCode::GATEWAY_TIMEOUT, "timeout", None);
    };
    let tail = || String::from_utf8_lossy(&err[err.len().saturating_sub(300)..]).into_owned();
    match parsed.finish() {
        Ok(reply) => Json(reply).into_response(),
        Err(Failure::MaxTurns) => cli_failed("max turns reached".into()),
        Err(Failure::NoResult) => {
            let code = status.ok().and_then(|s| s.code());
            cli_failed(format!("no result event (exit {code:?}): {}", tail()))
        }
    }
}

/// Delete files changed since the run start, then any empty dirs (best effort).
fn clean_sessions(dir: &Path, since: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            clean_sessions(&path, since);
            let _ = std::fs::remove_dir(&path);
        } else if entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t >= since)
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[derive(Serialize)]
struct Account {
    folder: String,
    label: String,
    sign_in_line: Option<String>,
}

async fn accounts(State(state): State<AppState>) -> Response {
    let entries = match std::fs::read_dir(&state.cfg.logins_dir) {
        Ok(entries) => entries,
        Err(e) => {
            let message = format!("cannot read the logins dir: {e}");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "logins_dir",
                Some(message),
            );
        }
    };
    let mut folders: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| valid_folder(name))
        .collect();
    folders.sort();
    let tasks: Vec<_> = folders
        .into_iter()
        .map(|folder| {
            let cfg = state.cfg.clone();
            tokio::spawn(async move {
                let dir = cfg.logins_dir.join(&folder);
                let label = std::fs::read_to_string(dir.join(".openjam-label"))
                    .map(|l| l.trim().to_string())
                    .ok()
                    .filter(|l| !l.is_empty())
                    .unwrap_or_else(|| folder.clone());
                let sign_in_line = sign_in_line(&cfg, &dir).await;
                Account {
                    folder,
                    label,
                    sign_in_line,
                }
            })
        })
        .collect();
    let mut list = Vec::with_capacity(tasks.len());
    for task in tasks {
        if let Ok(account) = task.await {
            list.push(account);
        }
    }
    Json(list).into_response()
}

/// The `Sign-in` line of `claude auth status --text`, or `None` on any failure.
async fn sign_in_line(cfg: &Config, dir: &Path) -> Option<String> {
    let tmp = temp_dir().ok()?;
    let mut cmd = claude_command(cfg, dir, tmp.path());
    cmd.args(["auth", "status", "--text"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = cmd.spawn().ok()?;
    let _group = Group(child.id());
    let out = tokio::time::timeout(AUTH_STATUS_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|l| l.starts_with("Sign-in"))
        .map(|l| l.trim_end().to_string())
}

async fn delete_account(
    State(state): State<AppState>,
    UrlPath(folder): UrlPath<String>,
) -> Response {
    let dir = match state.folder_dir(&folder) {
        Ok(dir) => dir,
        Err((status, code)) => return error(status, code, None),
    };
    let Some(_busy) = state.take(&folder) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "runner_busy", None);
    };
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let message = format!("cannot remove the folder: {e}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "remove_failed",
                Some(message),
            )
        }
    }
}
