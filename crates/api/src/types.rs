//! Contract types for the routes M0 implements. Later slices add their own.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct Me {
    pub user: MeUser,
    pub workspace_id: Uuid,
    pub project: Option<MeProject>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MeUser {
    pub id: Uuid,
    pub name: Option<String>,
    pub email: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MeProject {
    pub id: Uuid,
    pub domain: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProjectList {
    pub projects: Vec<ProjectSummary>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: Uuid,
    pub name: String,
    pub domain: String,
    pub site_url: String,
    /// `setup`, `active`, `paused` or `blocked`.
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateProject {
    pub name: String,
    /// Host name, for example `acme.com`.
    pub domain: String,
    #[serde(default)]
    pub site_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyList {
    pub keys: Vec<ApiKeySummary>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeySummary {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct IssueApiKey {
    /// A name to recognize the key later.
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssuedApiKey {
    pub id: Uuid,
    /// Shown once. Store it now; the server keeps only its hash.
    pub key: String,
    pub key_prefix: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Usage {
    /// Scans started in the current UTC day.
    pub scans_today: i64,
    /// `null` when no cap is set. The default is no cap.
    pub scan_cap_per_day: Option<u32>,
    pub cost_cents_today: f64,
    pub cost_cents_month: f64,
}
