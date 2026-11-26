//! Contract types for `/v1/analytics/*`. Field names follow the source API.

use super::metrics::MetricValue;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct OverviewQuery {
    /// Days in the window, 1 to 365. The default is 30.
    pub range_days: Option<i64>,
    /// The project to measure. The default is the newest project.
    pub project_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct PromptsQuery {
    pub range_days: Option<i64>,
    pub project_id: Option<Uuid>,
    /// Page size, 1 to 100. The default is 25.
    pub limit: Option<i64>,
    /// `nextCursor` of the page before.
    pub cursor: Option<String>,
    pub intent: Option<String>,
    pub topic: Option<String>,
    /// One engine, for example `openai`.
    pub platform: Option<String>,
    /// Text the prompt contains. Case is ignored.
    pub query: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MetricKind {
    Visibility,
    Competitors,
    Citations,
    Platforms,
    Topics,
    Sources,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct MetricsQuery {
    /// The metric family to flatten into rows.
    pub metric: MetricKind,
    pub range_days: Option<i64>,
    pub project_id: Option<Uuid>,
    /// One engine, for example `openai`. Applies to `visibility`, `competitors`, `citations` and `topics`.
    pub platform: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsProject {
    pub id: Uuid,
    pub brand_name: String,
    pub site_domain: String,
    pub status: String,
    pub bootstrap_completed_at: Option<DateTime<Utc>>,
    pub last_visibility_success_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsSelection {
    pub range: String,
    pub provider_ids: Vec<String>,
    pub segment_ids: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MetricDefinitions {
    pub percentages: &'static str,
    pub prompts: &'static str,
    pub visibility: &'static str,
    pub citations: &'static str,
    pub share_of_voice: &'static str,
    pub position: &'static str,
    pub scope: &'static str,
    pub counts: &'static str,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsWindow {
    pub label: String,
    pub comparison_label: String,
    pub start_date: String,
    pub end_date: String,
    pub answer_count: u64,
    pub provider_count: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsPlatform {
    pub id: String,
    pub label: String,
    pub display_name: String,
    /// `active` when the window has answers, `pending` when it has none.
    pub status: &'static str,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsBrand {
    pub key: String,
    pub name: String,
    pub domain: Option<String>,
    pub is_owned: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrendPoint {
    /// UTC day, `YYYY-MM-DD`.
    pub date: String,
    /// Mention rate of each brand on that day, by brand key.
    pub scores: BTreeMap<String, f64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrendSeries {
    pub brands: Vec<AnalyticsBrand>,
    pub points: Vec<TrendPoint>,
    pub unit: &'static str,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LeaderboardEntry {
    pub rank: Option<i64>,
    pub brand: AnalyticsBrand,
    pub visibility_score: Option<f64>,
    pub delta: Option<f64>,
    pub share_of_voice: Option<f64>,
    pub citation_share: Option<f64>,
    pub average_position: Option<f64>,
    pub mentions: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlatformBreakdownRow {
    pub provider_id: String,
    pub visibility_score: MetricValue,
    pub share_of_voice: MetricValue,
    pub citation_share: MetricValue,
    pub average_position: MetricValue,
    pub answer_count: u64,
    pub status: &'static str,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChangeFeedItem {
    /// `visibility_up` or `visibility_down`.
    pub kind: &'static str,
    pub provider_id: String,
    pub value: f64,
    pub delta: f64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicRanking {
    pub topic: String,
    pub prompt_count: u64,
    pub answer_count: u64,
    pub visibility_score: MetricValue,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsOverviewSection {
    pub visibility_score: MetricValue,
    pub share_of_voice: MetricValue,
    pub citation_share: MetricValue,
    pub average_position: MetricValue,
    pub trend: TrendSeries,
    pub leaderboard: Vec<LeaderboardEntry>,
    pub provider_breakdown: Vec<PlatformBreakdownRow>,
    pub change_feed: Vec<ChangeFeedItem>,
    pub topic_rankings: Vec<TopicRanking>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsOverview {
    pub project: AnalyticsProject,
    pub selection: AnalyticsSelection,
    pub metric_source: &'static str,
    pub metric_definitions: MetricDefinitions,
    pub window: AnalyticsWindow,
    pub providers: Vec<AnalyticsPlatform>,
    pub overview: AnalyticsOverviewSection,
    pub tracked_prompts: u64,
    pub prompts_tracked: u64,
    pub prompts_measured: u64,
    pub last_scan_at: Option<DateTime<Utc>>,
    pub next_scan_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptMetrics {
    pub visibility_rank: Option<i64>,
    pub visibility_score: MetricValue,
    pub share_of_voice: MetricValue,
    pub average_position: MetricValue,
    pub citation_share: MetricValue,
    pub citation_rank: Option<i64>,
    pub executions: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptRow {
    pub id: Uuid,
    pub text: String,
    pub topic: Option<String>,
    pub persona: Option<String>,
    pub intent: Option<String>,
    pub metrics: PromptMetrics,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptFilters {
    pub intent: Option<String>,
    pub topic: Option<String>,
    pub platform: Option<String>,
    pub query: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub limit: i64,
    pub has_next_page: bool,
    pub next_cursor: Option<String>,
    pub total_count: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptAnalytics {
    pub project: AnalyticsProject,
    pub window: AnalyticsWindow,
    pub providers: Vec<AnalyticsPlatform>,
    pub metric_definitions: MetricDefinitions,
    pub prompts: Vec<PromptRow>,
    pub filters: PromptFilters,
    pub page_info: PageInfo,
}

/// One row of a metric family. Each family fills the fields that apply to it.
#[derive(Debug, Default, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricRow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_owned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_of_voice: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation_share: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mentions: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_position: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_count: Option<u64>,
    /// Citations of the domain (`sources`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citations: Option<u64>,
    /// Answers that cite the domain (`sources`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answers: Option<u64>,
    /// Answers that cite the domain, as a percent of all answers (`sources`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_share: Option<f64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MetricRows {
    /// The family asked for: `visibility`, `competitors`, `citations`, `platforms`, `topics` or `sources`.
    pub metric: MetricKind,
    pub rows: Vec<MetricRow>,
}
