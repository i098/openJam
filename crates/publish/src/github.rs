//! The GitHub App client: an app token signs in, the installation token acts on one repository.
//!
//! It opens branches and pull requests. It never pushes to the base branch and never merges.

use base64::{Engine, engine::general_purpose::STANDARD};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Method;
use serde::Serialize;
use serde_json::{Value, json};
use std::{env, sync::LazyLock, time::Duration};

pub const DEFAULT_API: &str = "https://api.github.com";

#[derive(Debug, thiserror::Error)]
pub enum GithubError {
    #[error(
        "the GitHub App is not set up: set OPENJAM_GITHUB_APP_ID and OPENJAM_GITHUB_APP_PRIVATE_KEY (or OPENJAM_GITHUB_APP_PRIVATE_KEY_FILE)"
    )]
    NotConfigured,
    #[error("the GitHub App private key is not valid: {0}")]
    Key(String),
    #[error("GitHub answered {status}: {message}")]
    Api { status: u16, message: String },
    #[error("could not reach GitHub: {0}")]
    Network(String),
}

impl GithubError {
    /// A retry cannot fix it: the key, the permissions or the request is wrong.
    pub fn is_permanent(&self) -> bool {
        match self {
            Self::NotConfigured | Self::Key(_) => true,
            Self::Api { status, message } => {
                (400..500).contains(status)
                    && !matches!(status, 408 | 429)
                    && !message.to_lowercase().contains("rate limit")
            }
            Self::Network(_) => false,
        }
    }

    fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status, .. } => Some(*status),
            _ => None,
        }
    }
}

type Result<T, E = GithubError> = std::result::Result<T, E>;

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("openJam")
        .timeout(Duration::from_secs(30))
        .build()
        .expect("the HTTP client builds")
});

async fn call(method: Method, url: &str, token: &str, body: Option<Value>) -> Result<Value> {
    let mut request = CLIENT
        .request(method, url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|e| GithubError::Network(e.without_url().to_string()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| GithubError::Network(e.without_url().to_string()))?;
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(value);
    }
    let message = value["message"]
        .as_str()
        .map_or_else(|| text.chars().take(200).collect(), str::to_owned);
    Err(GithubError::Api {
        status: status.as_u16(),
        message,
    })
}

#[derive(Serialize)]
struct Claims {
    iat: i64,
    exp: i64,
    iss: String,
}

/// The GitHub App of this install.
pub struct App {
    id: String,
    key: EncodingKey,
    algorithm: Algorithm,
    api: String,
}

impl App {
    /// `None` when `OPENJAM_GITHUB_APP_ID` is not set: publishing to GitHub is then off.
    pub fn from_env() -> Result<Option<Self>> {
        let var = |name: &str| env::var(name).ok().filter(|v| !v.trim().is_empty());
        let Some(id) = var("OPENJAM_GITHUB_APP_ID") else {
            return Ok(None);
        };
        let pem = match (
            var("OPENJAM_GITHUB_APP_PRIVATE_KEY"),
            var("OPENJAM_GITHUB_APP_PRIVATE_KEY_FILE"),
        ) {
            (Some(pem), _) => pem.replace("\\n", "\n"),
            (None, Some(path)) => std::fs::read_to_string(&path)
                .map_err(|e| GithubError::Key(format!("cannot read {path}: {e}")))?,
            (None, None) => return Err(GithubError::NotConfigured),
        };
        let key = EncodingKey::from_rsa_pem(pem.as_bytes())
            .map_err(|e| GithubError::Key(e.to_string()))?;
        let api = var("OPENJAM_GITHUB_API_URL").unwrap_or_else(|| DEFAULT_API.into());
        Ok(Some(Self::new(id, key, Algorithm::RS256, &api)))
    }

    pub fn new(id: String, key: EncodingKey, algorithm: Algorithm, api: &str) -> Self {
        Self {
            id,
            key,
            algorithm,
            api: api.trim_end_matches('/').to_owned(),
        }
    }

    fn jwt(&self) -> Result<String> {
        let now = chrono::Utc::now().timestamp();
        let claims = Claims {
            // GitHub allows a clock that runs a minute behind, and tokens up to ten minutes.
            iat: now - 60,
            exp: now + 540,
            iss: self.id.clone(),
        };
        jsonwebtoken::encode(&Header::new(self.algorithm), &claims, &self.key)
            .map_err(|e| GithubError::Key(e.to_string()))
    }

    /// The installation of this app on `owner/name`. `None`: the app is not installed there.
    pub async fn installation(&self, owner: &str, name: &str) -> Result<Option<Installation>> {
        let url = format!("{}/repos/{owner}/{name}/installation", self.api);
        match call(Method::GET, &url, &self.jwt()?, None).await {
            Ok(v) => Ok(v["id"].as_u64().map(|id| Installation {
                id,
                can_open_prs: v["permissions"]["contents"] == "write"
                    && v["permissions"]["pull_requests"] == "write",
            })),
            Err(e) if e.status() == Some(404) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// A client for one repository, with a token that lasts about an hour.
    pub async fn repo(&self, installation_id: u64, owner: &str, name: &str) -> Result<Repo> {
        let url = format!(
            "{}/app/installations/{installation_id}/access_tokens",
            self.api
        );
        let v = call(Method::POST, &url, &self.jwt()?, Some(json!({}))).await?;
        let token = v["token"]
            .as_str()
            .ok_or_else(|| GithubError::Api {
                status: 502,
                message: "no installation token in the answer".into(),
            })?
            .to_owned();
        Ok(Repo {
            api: self.api.clone(),
            token,
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }
}

/// The app's installation on one repository.
#[derive(Debug)]
pub struct Installation {
    pub id: u64,
    /// The installation may write contents and pull requests.
    pub can_open_prs: bool,
}

/// One pull request as the delivery reads it.
#[derive(Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub number: i32,
    pub url: String,
    pub open: bool,
    pub merged: bool,
}

impl PullRequest {
    fn from(v: &Value) -> Option<Self> {
        Some(Self {
            number: i32::try_from(v["number"].as_i64()?).ok()?,
            url: v["html_url"].as_str()?.to_owned(),
            open: v["state"].as_str()? == "open",
            merged: v["merged"].as_bool().unwrap_or(false) || !v["merged_at"].is_null(),
        })
    }
}

pub struct Repo {
    api: String,
    token: String,
    owner: String,
    name: String,
}

impl Repo {
    fn url(&self, tail: &str) -> String {
        format!("{}/repos/{}/{}/{tail}", self.api, self.owner, self.name)
    }

    async fn get(&self, tail: &str) -> Result<Value> {
        call(Method::GET, &self.url(tail), &self.token, None).await
    }

    /// The commit a branch points at. `None`: the branch does not exist.
    pub async fn branch_sha(&self, branch: &str) -> Result<Option<String>> {
        match self.get(&format!("git/ref/heads/{branch}")).await {
            Ok(v) => Ok(v["object"]["sha"].as_str().map(str::to_owned)),
            Err(e) if e.status() == Some(404) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Create `branch` at `sha`. A branch that exists already is kept; this makes a retry safe.
    pub async fn ensure_branch(&self, branch: &str, sha: &str) -> Result<()> {
        let body = json!({"ref": format!("refs/heads/{branch}"), "sha": sha});
        match call(Method::POST, &self.url("git/refs"), &self.token, Some(body)).await {
            Err(e) if e.status() == Some(422) => Ok(()),
            other => other.map(|_| ()),
        }
    }

    /// File names in a folder of `branch`. A folder that does not exist has none.
    pub async fn list_dir(&self, branch: &str, path: &str) -> Result<Vec<String>> {
        match self.get(&format!("contents/{path}?ref={branch}")).await {
            Ok(Value::Array(items)) => Ok(items
                .iter()
                .filter_map(|i| i["name"].as_str().map(str::to_owned))
                .collect()),
            Ok(_) => Err(GithubError::Api {
                status: 400,
                message: format!("{path} is a file, not a folder"),
            }),
            Err(e) if e.status() == Some(404) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Write `path` on `branch`: a new file, or a new version of the file there.
    pub async fn put_file(
        &self,
        branch: &str,
        path: &str,
        text: &str,
        message: &str,
    ) -> Result<()> {
        let sha = match self.get(&format!("contents/{path}?ref={branch}")).await {
            Ok(v) => v["sha"].as_str().map(str::to_owned),
            Err(e) if e.status() == Some(404) => None,
            Err(e) => return Err(e),
        };
        let mut body = json!({
            "message": message,
            "content": STANDARD.encode(text),
            "branch": branch,
        });
        if let Some(sha) = sha {
            body["sha"] = json!(sha);
        }
        call(
            Method::PUT,
            &self.url(&format!("contents/{path}")),
            &self.token,
            Some(body),
        )
        .await
        .map(|_| ())
    }

    /// Open a pull request from `head` into `base`. One that exists already is returned.
    pub async fn open_pr(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<PullRequest> {
        let payload = json!({"title": title, "head": head, "base": base, "body": body});
        match call(Method::POST, &self.url("pulls"), &self.token, Some(payload)).await {
            Ok(v) => PullRequest::from(&v).ok_or_else(bad_pr),
            Err(e) if e.status() == Some(422) => {
                let found = self
                    .get(&format!("pulls?state=all&head={}:{head}", self.owner))
                    .await?;
                found
                    .as_array()
                    .and_then(|list| list.first())
                    .and_then(PullRequest::from)
                    .ok_or(e)
            }
            Err(e) => Err(e),
        }
    }

    pub async fn pr(&self, number: i32) -> Result<PullRequest> {
        PullRequest::from(&self.get(&format!("pulls/{number}")).await?).ok_or_else(bad_pr)
    }

    /// Close an open pull request and delete its branch.
    pub async fn withdraw(&self, number: i32, branch: &str) -> Result<()> {
        let close = json!({"state": "closed"});
        call(
            Method::PATCH,
            &self.url(&format!("pulls/{number}")),
            &self.token,
            Some(close),
        )
        .await?;
        match call(
            Method::DELETE,
            &self.url(&format!("git/refs/heads/{branch}")),
            &self.token,
            None,
        )
        .await
        {
            Err(e) if e.status() == Some(422) || e.status() == Some(404) => Ok(()),
            other => other.map(|_| ()),
        }
    }
}

fn bad_pr() -> GithubError {
    GithubError::Api {
        status: 502,
        message: "GitHub sent a pull request without number, state or address".into(),
    }
}
