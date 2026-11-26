//! `/v1/analytics/*`: visibility, share of voice, citations and position, per engine and over time.

pub mod metrics;
mod queries;
pub mod types;

use crate::error::{ApiError, Error, Result};
use crate::{auth::Principal, error::ApiErrorBody, state::AppState};
use axum::{
    Json,
    extract::{FromRequestParts, Query, State},
};
use chrono::{DateTime, Duration, Utc};
use metrics::*;
use queries::{Brand, BrandTally, Pair, Scope, Split};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use types::*;
use uuid::Uuid;

const DEFAULT_RANGE_DAYS: i64 = 30;
const TREND_BRANDS: usize = 5;

/// `Query` that fails with the `/v1` error body.
pub struct Q<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequestParts<S> for Q<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut axum::http::request::Parts, state: &S) -> Result<Self> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|e| ApiError::bad(e.body_text()))
    }
}

struct Project {
    id: Uuid,
    name: String,
    domain: String,
    status: String,
    first_scan_at: Option<DateTime<Utc>>,
    last_scan_at: Option<DateTime<Utc>>,
    next_scan_at: Option<DateTime<Utc>>,
    platforms: Vec<String>,
}

impl Project {
    fn api(&self) -> AnalyticsProject {
        AnalyticsProject {
            id: self.id,
            brand_name: self.name.clone(),
            site_domain: self.domain.clone(),
            status: self.status.clone(),
            bootstrap_completed_at: self.first_scan_at,
            last_visibility_success_at: self.last_scan_at,
        }
    }

    fn brand(&self) -> AnalyticsBrand {
        AnalyticsBrand {
            key: slug(&self.name),
            name: self.name.clone(),
            domain: Some(self.domain.clone()),
            is_owned: true,
        }
    }
}

async fn find_project(
    state: &AppState,
    principal: &Principal,
    project_id: Option<Uuid>,
) -> Result<Project> {
    type Row = (
        Uuid,
        String,
        String,
        String,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        Vec<String>,
    );
    let row: Option<Row> = sqlx::query_as(
        "SELECT id, name, domain, status, first_scan_at, last_scan_at, next_scan_at, platforms
         FROM projects
         WHERE workspace_id = $1 AND archived_at IS NULL AND ($2::uuid IS NULL OR id = $2)
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(principal.workspace_id())
    .bind(project_id)
    .fetch_optional(&state.pool)
    .await?;
    let (id, name, domain, status, first_scan_at, last_scan_at, next_scan_at, platforms) =
        row.ok_or(Error::NotFound("project"))?;
    Ok(Project {
        id,
        name,
        domain,
        status,
        first_scan_at,
        last_scan_at,
        next_scan_at,
        platforms,
    })
}

/// The current window and the window before it, each `days` long.
struct Window {
    days: i64,
    cur_start: DateTime<Utc>,
    prev_start: DateTime<Utc>,
    end: DateTime<Utc>,
}

impl Window {
    fn new(range_days: Option<i64>, now: DateTime<Utc>) -> Result<Self> {
        let days = range_days.unwrap_or(DEFAULT_RANGE_DAYS);
        if !(1..=365).contains(&days) {
            return Err(ApiError::bad("rangeDays must be 1 to 365"));
        }
        Ok(Self {
            days,
            cur_start: now - Duration::days(days),
            prev_start: now - Duration::days(2 * days),
            end: now,
        })
    }

    fn scope(&self, project: &Project) -> Scope {
        Scope {
            project_id: project.id,
            prev_start: self.prev_start,
            cur_start: self.cur_start,
            end: self.end,
            platform: None,
            intent: None,
            topic: None,
            query: None,
        }
    }

    fn api(&self, answers: u64, providers: u64) -> AnalyticsWindow {
        AnalyticsWindow {
            label: format!("Last {} days", self.days),
            comparison_label: format!("Previous {} days", self.days),
            start_date: self.cur_start.format("%Y-%m-%d").to_string(),
            end_date: self.end.format("%Y-%m-%d").to_string(),
            answer_count: answers,
            provider_count: providers,
        }
    }
}

fn definitions() -> MetricDefinitions {
    MetricDefinitions {
        percentages: "Percentages have two decimals and run from 0 to 100.",
        prompts: "Only tracked prompts that count toward the score are measured.",
        visibility: "Visibility score is the percent of answers that mention the brand.",
        citations: "Citation share is the percent of answers that cite the brand's own domain.",
        share_of_voice: "Share of voice is the brand's answer mentions as a percent of all brand mentions, the brand's and its competitors'.",
        position: "Average position is the mean list position in the answers that rank the brand. A lower number is better.",
        scope: "Each metric covers the window. The delta is the change against the window before.",
        counts: "Only answers the judge has assessed are counted.",
    }
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn display_name(platform: &str) -> String {
    match platform {
        "openai" => "OpenAI",
        "gemini" => "Gemini",
        "perplexity" => "Perplexity",
        "claude" => "Claude",
        other => other,
    }
    .to_string()
}

fn status(answers: u64) -> &'static str {
    if answers > 0 { "active" } else { "pending" }
}

fn competitor_brand(brand: &Brand) -> AnalyticsBrand {
    AnalyticsBrand {
        key: slug(&brand.name),
        name: brand.name.clone(),
        domain: brand.domain.clone(),
        is_owned: false,
    }
}

/// A competitor's counts as answer counts, so the rate functions apply to it.
fn brand_tally(brand: &BrandTally, answers: u64) -> AnswerTally {
    AnswerTally {
        answers,
        mentioned: brand.mentions,
        cited: brand.cited,
        position_sum: brand.position_sum,
        position_count: brand.position_count,
        ..AnswerTally::default()
    }
}

fn competitor_mentions(brands: &[Brand], current: bool) -> u64 {
    brands
        .iter()
        .map(|b| {
            if current {
                b.tally.cur.mentions
            } else {
                b.tally.prev.mentions
            }
        })
        .sum()
}

fn sov_value(own: &Pair<AnswerTally>, brands: &[Brand]) -> MetricValue {
    metric_value(
        share_of_voice(own.cur.mentioned, competitor_mentions(brands, true)),
        share_of_voice(own.prev.mentioned, competitor_mentions(brands, false)),
    )
}

fn leaderboard(
    project: &Project,
    own: &Pair<AnswerTally>,
    brands: &[Brand],
) -> Vec<LeaderboardEntry> {
    let all = own.cur.mentioned + competitor_mentions(brands, true);
    let mut entries = vec![LeaderboardEntry {
        rank: None,
        brand: project.brand(),
        visibility_score: mention_rate(&own.cur),
        delta: metric_value(mention_rate(&own.cur), mention_rate(&own.prev)).delta,
        share_of_voice: share_of_voice(own.cur.mentioned, all - own.cur.mentioned),
        citation_share: citation_rate(&own.cur),
        average_position: average_position(&own.cur),
        mentions: own.cur.mentioned as i64,
    }];
    for brand in brands.iter().filter(|b| b.tally.cur.mentions > 0) {
        let cur = brand_tally(&brand.tally.cur, own.cur.answers);
        let prev = brand_tally(&brand.tally.prev, own.prev.answers);
        entries.push(LeaderboardEntry {
            rank: None,
            brand: competitor_brand(brand),
            visibility_score: mention_rate(&cur),
            delta: metric_value(mention_rate(&cur), mention_rate(&prev)).delta,
            share_of_voice: share_of_voice(cur.mentioned, all - cur.mentioned),
            citation_share: if brand.domain.is_some() {
                citation_rate(&cur)
            } else {
                None
            },
            average_position: average_position(&cur),
            mentions: cur.mentioned as i64,
        });
    }
    entries.sort_by(|a, b| {
        b.visibility_score
            .partial_cmp(&a.visibility_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.mentions.cmp(&a.mentions))
            .then(a.brand.name.cmp(&b.brand.name))
    });
    let scores: Vec<_> = entries.iter().map(|e| e.visibility_score).collect();
    for (entry, rank) in entries.iter_mut().zip(rank_desc(&scores)) {
        entry.rank = rank;
    }
    entries
}

/// The own brand's counts and the competitors' over the whole window.
async fn totals(state: &AppState, scope: &Scope) -> Result<(Pair<AnswerTally>, Vec<Brand>)> {
    let own = queries::tallies(&state.pool, scope, Split::All).await?;
    let mut brands = queries::competitors(&state.pool, scope, Split::All).await?;
    Ok((
        own.into_values().next().unwrap_or_default(),
        brands.remove("all").unwrap_or_default(),
    ))
}

async fn platform_rows(
    state: &AppState,
    scope: &Scope,
    project: &Project,
) -> Result<Vec<PlatformBreakdownRow>> {
    let tallies = queries::tallies(&state.pool, scope, Split::Platform).await?;
    let brands = queries::competitors(&state.pool, scope, Split::Platform).await?;
    let mut names: Vec<String> = project.platforms.clone();
    for key in tallies.keys() {
        if !names.contains(key) {
            names.push(key.clone());
        }
    }
    Ok(names
        .into_iter()
        .map(|name| {
            let pair = tallies.get(&name).cloned().unwrap_or_default();
            let comps = brands.get(&name).map(Vec::as_slice).unwrap_or(&[]);
            PlatformBreakdownRow {
                visibility_score: metric_value(mention_rate(&pair.cur), mention_rate(&pair.prev)),
                share_of_voice: sov_value(&pair, comps),
                citation_share: metric_value(citation_rate(&pair.cur), citation_rate(&pair.prev)),
                average_position: metric_value(
                    average_position(&pair.cur),
                    average_position(&pair.prev),
                ),
                answer_count: pair.cur.answers,
                status: status(pair.cur.answers),
                provider_id: name,
            }
        })
        .collect())
}

fn providers(rows: &[PlatformBreakdownRow]) -> Vec<AnalyticsPlatform> {
    rows.iter()
        .map(|row| AnalyticsPlatform {
            id: row.provider_id.clone(),
            label: row.provider_id.clone(),
            display_name: display_name(&row.provider_id),
            status: row.status,
        })
        .collect()
}

/// Metrics for each UTC day of the current window that has answers.
struct Day {
    date: String,
    own: AnswerTally,
    brands: Vec<Brand>,
}

async fn days(state: &AppState, scope: &Scope) -> Result<Vec<Day>> {
    let tallies = queries::tallies(&state.pool, scope, Split::Day).await?;
    let mut brands = queries::competitors(&state.pool, scope, Split::Day).await?;
    Ok(tallies
        .into_iter()
        .filter(|(_, pair)| pair.cur.answers > 0)
        .map(|(date, pair)| {
            let brands = brands
                .remove(&date)
                .unwrap_or_default()
                .into_iter()
                .filter(|b| b.tally.cur.mentions > 0)
                .collect();
            Day {
                date,
                own: pair.cur,
                brands,
            }
        })
        .collect())
}

/// `GET /v1/analytics/overview`
#[utoipa::path(
    get, path = "/v1/analytics/overview", tag = "Analytics", operation_id = "get_analytics_overview",
    summary = "Visibility, share of voice, citations and position for a window",
    security(("bearerAuth" = [])), params(OverviewQuery),
    responses(
        (status = 200, body = AnalyticsOverview),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn get_analytics_overview(
    State(state): State<AppState>,
    principal: Principal,
    Q(q): Q<OverviewQuery>,
) -> Result<Json<AnalyticsOverview>> {
    let project = find_project(&state, &principal, q.project_id).await?;
    let window = Window::new(q.range_days, Utc::now())?;
    let scope = window.scope(&project);

    let (own, brands) = totals(&state, &scope).await?;
    let platforms = platform_rows(&state, &scope, &project).await?;
    let prompts = queries::tallies(&state.pool, &scope, Split::Prompt).await?;
    let topics = queries::tallies(&state.pool, &scope, Split::Topic).await?;
    let prompt_counts = queries::prompt_counts(&state.pool, project.id).await?;
    let trend = trend(&project, &brands, days(&state, &scope).await?);

    let mut topic_rankings: Vec<TopicRanking> = prompt_counts
        .iter()
        .map(|(topic, count)| {
            let pair = topics.get(topic).cloned().unwrap_or_default();
            TopicRanking {
                topic: topic.clone(),
                prompt_count: *count,
                answer_count: pair.cur.answers,
                visibility_score: metric_value(mention_rate(&pair.cur), mention_rate(&pair.prev)),
            }
        })
        .collect();
    topic_rankings.sort_by(|a, b| {
        b.visibility_score
            .value
            .partial_cmp(&a.visibility_score.value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.topic.cmp(&b.topic))
    });

    let mut change_feed: Vec<ChangeFeedItem> = platforms
        .iter()
        .filter_map(|row| {
            let delta = row.visibility_score.delta.filter(|d| *d != 0.0)?;
            Some(ChangeFeedItem {
                kind: if delta > 0.0 {
                    "visibility_up"
                } else {
                    "visibility_down"
                },
                provider_id: row.provider_id.clone(),
                value: row.visibility_score.value?,
                delta,
            })
        })
        .collect();
    change_feed.sort_by(|a, b| {
        b.delta
            .abs()
            .total_cmp(&a.delta.abs())
            .then(a.provider_id.cmp(&b.provider_id))
    });

    Ok(Json(AnalyticsOverview {
        project: project.api(),
        selection: AnalyticsSelection {
            range: format!("{}d", window.days),
            provider_ids: platforms.iter().map(|p| p.provider_id.clone()).collect(),
            segment_ids: vec![],
        },
        metric_source: "analytics_dashboard",
        metric_definitions: definitions(),
        window: window.api(
            own.cur.answers,
            platforms.iter().filter(|p| p.answer_count > 0).count() as u64,
        ),
        providers: providers(&platforms),
        overview: AnalyticsOverviewSection {
            visibility_score: metric_value(mention_rate(&own.cur), mention_rate(&own.prev)),
            share_of_voice: sov_value(&own, &brands),
            citation_share: metric_value(citation_rate(&own.cur), citation_rate(&own.prev)),
            average_position: metric_value(average_position(&own.cur), average_position(&own.prev)),
            trend,
            leaderboard: leaderboard(&project, &own, &brands),
            provider_breakdown: platforms,
            change_feed,
            topic_rankings,
        },
        tracked_prompts: prompt_counts.values().sum(),
        prompts_tracked: prompt_counts.values().sum(),
        prompts_measured: prompts.values().filter(|p| p.cur.answers > 0).count() as u64,
        last_scan_at: project.last_scan_at,
        next_scan_at: project.next_scan_at,
    }))
}

/// Mention rate per day for the own brand and the competitors with most mentions.
fn trend(project: &Project, brands: &[Brand], days: Vec<Day>) -> TrendSeries {
    let mut top: Vec<&Brand> = brands.iter().filter(|b| b.tally.cur.mentions > 0).collect();
    top.sort_by(|a, b| {
        b.tally
            .cur
            .mentions
            .cmp(&a.tally.cur.mentions)
            .then(a.name.cmp(&b.name))
    });
    top.truncate(TREND_BRANDS);
    let mut series = vec![project.brand()];
    series.extend(top.iter().map(|b| competitor_brand(b)));
    let points = days
        .into_iter()
        .map(|day| {
            let mut scores = BTreeMap::new();
            if let Some(score) = mention_rate(&day.own) {
                scores.insert(slug(&project.name), score);
            }
            for brand in &top {
                let mentions = day
                    .brands
                    .iter()
                    .find(|b| b.name == brand.name)
                    .map_or(0, |b| b.tally.cur.mentions);
                if let Some(score) = percent(mentions, day.own.answers) {
                    scores.insert(slug(&brand.name), score);
                }
            }
            TrendPoint {
                date: day.date,
                scores,
            }
        })
        .collect();
    TrendSeries {
        brands: series,
        points,
        unit: "percent",
    }
}

/// `GET /v1/analytics/prompts`
#[utoipa::path(
    get, path = "/v1/analytics/prompts", tag = "Analytics", operation_id = "list_prompt_analytics",
    summary = "Metrics for each tracked prompt",
    security(("bearerAuth" = [])), params(PromptsQuery),
    responses(
        (status = 200, body = PromptAnalytics),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn list_prompt_analytics(
    State(state): State<AppState>,
    principal: Principal,
    Q(q): Q<PromptsQuery>,
) -> Result<Json<PromptAnalytics>> {
    let limit = q.limit.unwrap_or(25);
    if !(1..=100).contains(&limit) {
        return Err(ApiError::bad("limit must be 1 to 100"));
    }
    let offset: usize = match q.cursor.as_deref() {
        None => 0,
        Some(cursor) => cursor
            .parse()
            .map_err(|_| ApiError::bad("cursor is not valid"))?,
    };
    let project = find_project(&state, &principal, q.project_id).await?;
    let window = Window::new(q.range_days, Utc::now())?;
    let mut scope = window.scope(&project);
    scope.platform = q.platform.clone();
    scope.intent = q.intent.clone();
    scope.topic = q.topic.clone();
    scope.query = q.query.clone().filter(|t| !t.is_empty());

    let infos = queries::prompts(&state.pool, &scope).await?;
    let tallies = queries::tallies(&state.pool, &scope, Split::Prompt).await?;
    let brands = queries::competitors(&state.pool, &scope, Split::Prompt).await?;
    let platforms = platform_rows(&state, &scope, &project).await?;
    let answer_count: u64 = tallies.values().map(|p| p.cur.answers).sum();

    let pairs: Vec<Pair<AnswerTally>> = infos
        .iter()
        .map(|p| tallies.get(&p.id.to_string()).cloned().unwrap_or_default())
        .collect();
    let visibility: Vec<_> = pairs.iter().map(|p| mention_rate(&p.cur)).collect();
    let citation: Vec<_> = pairs.iter().map(|p| citation_rate(&p.cur)).collect();
    let (visibility_ranks, citation_ranks) = (rank_desc(&visibility), rank_desc(&citation));

    let mut rows: Vec<PromptRow> = infos
        .into_iter()
        .zip(pairs)
        .enumerate()
        .map(|(i, (info, pair))| {
            let comps = brands
                .get(&info.id.to_string())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            PromptRow {
                id: info.id,
                text: info.text,
                topic: info.topic,
                persona: info.persona,
                intent: info.intent,
                metrics: PromptMetrics {
                    visibility_rank: visibility_ranks[i],
                    visibility_score: metric_value(
                        mention_rate(&pair.cur),
                        mention_rate(&pair.prev),
                    ),
                    share_of_voice: sov_value(&pair, comps),
                    average_position: metric_value(
                        average_position(&pair.cur),
                        average_position(&pair.prev),
                    ),
                    citation_share: metric_value(
                        citation_rate(&pair.cur),
                        citation_rate(&pair.prev),
                    ),
                    citation_rank: citation_ranks[i],
                    executions: pair.cur.answers,
                },
            }
        })
        .collect();
    // Best visibility first; prompts with no data last. The sort is stable, so ties keep creation order.
    rows.sort_by_key(|r| r.metrics.visibility_rank.unwrap_or(i64::MAX));

    let total = rows.len();
    let page: Vec<PromptRow> = rows.into_iter().skip(offset).take(limit as usize).collect();
    let next = offset + page.len();
    let has_next = next < total;
    Ok(Json(PromptAnalytics {
        project: project.api(),
        window: window.api(
            answer_count,
            platforms.iter().filter(|p| p.answer_count > 0).count() as u64,
        ),
        providers: providers(&platforms),
        metric_definitions: definitions(),
        prompts: page,
        filters: PromptFilters {
            intent: q.intent,
            topic: q.topic,
            platform: q.platform,
            query: q.query,
        },
        page_info: PageInfo {
            limit,
            has_next_page: has_next,
            next_cursor: has_next.then(|| next.to_string()),
            total_count: total as u64,
        },
    }))
}

/// `GET /v1/keywords`: the `topics` metric under its own name.
#[utoipa::path(
    get, path = "/v1/keywords", tag = "Keywords", operation_id = "list_keywords",
    summary = "Keyword and topic visibility",
    security(("bearerAuth" = [])), params(OverviewQuery),
    responses(
        (status = 200, body = MetricRows),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn list_keywords(
    State(state): State<AppState>,
    principal: Principal,
    Q(q): Q<OverviewQuery>,
) -> Result<Json<MetricRows>> {
    let project = find_project(&state, &principal, q.project_id).await?;
    let window = Window::new(q.range_days, Utc::now())?;
    let rows = topic_rows(&state, &window.scope(&project), &project).await?;
    Ok(Json(MetricRows {
        metric: MetricKind::Topics,
        rows,
    }))
}

/// `GET /v1/analytics/metrics`
#[utoipa::path(
    get, path = "/v1/analytics/metrics", tag = "Analytics", operation_id = "list_analytics_metrics",
    summary = "One metric family as flat rows, for charts",
    security(("bearerAuth" = [])), params(MetricsQuery),
    responses(
        (status = 200, body = MetricRows),
        (status = 400, body = ApiErrorBody),
        (status = 401, body = ApiErrorBody),
        (status = 404, body = ApiErrorBody)
    )
)]
pub async fn list_analytics_metrics(
    State(state): State<AppState>,
    principal: Principal,
    Q(q): Q<MetricsQuery>,
) -> Result<Json<MetricRows>> {
    let project = find_project(&state, &principal, q.project_id).await?;
    let window = Window::new(q.range_days, Utc::now())?;
    let mut scope = window.scope(&project);
    scope.platform = q.platform.clone();

    let rows = match q.metric {
        MetricKind::Visibility => visibility_rows(&state, &scope, &project).await?,
        MetricKind::Competitors => {
            let (own, brands) = totals(&state, &scope).await?;
            leaderboard(&project, &own, &brands)
                .into_iter()
                .map(|e| MetricRow {
                    brand_key: Some(e.brand.key),
                    brand: Some(e.brand.name),
                    domain: e.brand.domain,
                    is_owned: Some(e.brand.is_owned),
                    rank: e.rank,
                    visibility_score: e.visibility_score,
                    share_of_voice: e.share_of_voice,
                    citation_share: e.citation_share,
                    average_position: e.average_position,
                    mentions: Some(e.mentions),
                    answer_count: Some(own.cur.answers),
                    ..MetricRow::default()
                })
                .collect()
        }
        MetricKind::Citations => days(&state, &scope)
            .await?
            .into_iter()
            .map(|day| MetricRow {
                date: Some(day.date),
                citation_share: citation_rate(&day.own),
                citations: Some(day.own.cited),
                answer_count: Some(day.own.answers),
                ..MetricRow::default()
            })
            .collect(),
        MetricKind::Platforms => {
            scope.platform = None;
            platform_rows(&state, &scope, &project)
                .await?
                .into_iter()
                .map(|p| MetricRow {
                    platform: Some(p.provider_id),
                    visibility_score: p.visibility_score.value,
                    share_of_voice: p.share_of_voice.value,
                    citation_share: p.citation_share.value,
                    average_position: p.average_position.value,
                    answer_count: Some(p.answer_count),
                    status: Some(p.status.to_string()),
                    ..MetricRow::default()
                })
                .collect()
        }
        MetricKind::Topics => topic_rows(&state, &scope, &project).await?,
        MetricKind::Sources => {
            let (own, _) = totals(&state, &scope).await?;
            queries::sources(&state.pool, &scope)
                .await?
                .into_iter()
                .map(|s| MetricRow {
                    is_owned: Some(
                        s.domain == project.domain
                            || s.domain.ends_with(&format!(".{}", project.domain)),
                    ),
                    answer_share: percent(s.answers, own.cur.answers),
                    citations: Some(s.citations),
                    answers: Some(s.answers),
                    domain: Some(s.domain),
                    ..MetricRow::default()
                })
                .collect()
        }
    };
    Ok(Json(MetricRows {
        metric: q.metric,
        rows,
    }))
}

async fn topic_rows(state: &AppState, scope: &Scope, project: &Project) -> Result<Vec<MetricRow>> {
    let topics = queries::tallies(&state.pool, scope, Split::Topic).await?;
    let counts = queries::prompt_counts(&state.pool, project.id).await?;
    Ok(counts
        .into_iter()
        .map(|(topic, prompt_count)| {
            let cur = topics.get(&topic).map(|p| p.cur).unwrap_or_default();
            MetricRow {
                topic: Some(topic),
                prompt_count: Some(prompt_count),
                answer_count: Some(cur.answers),
                visibility_score: mention_rate(&cur),
                citation_share: citation_rate(&cur),
                average_position: average_position(&cur),
                ..MetricRow::default()
            }
        })
        .collect())
}

async fn visibility_rows(
    state: &AppState,
    scope: &Scope,
    project: &Project,
) -> Result<Vec<MetricRow>> {
    let mut rows = Vec::new();
    for day in days(state, scope).await? {
        let all = day.own.mentioned + day.brands.iter().map(|b| b.tally.cur.mentions).sum::<u64>();
        let own = project.brand();
        rows.push(MetricRow {
            date: Some(day.date.clone()),
            brand_key: Some(own.key),
            brand: Some(own.name),
            domain: own.domain,
            is_owned: Some(true),
            visibility_score: mention_rate(&day.own),
            share_of_voice: share_of_voice(day.own.mentioned, all - day.own.mentioned),
            average_position: average_position(&day.own),
            mentions: Some(day.own.mentioned as i64),
            answer_count: Some(day.own.answers),
            ..MetricRow::default()
        });
        for brand in &day.brands {
            let cur = brand_tally(&brand.tally.cur, day.own.answers);
            let b = competitor_brand(brand);
            rows.push(MetricRow {
                date: Some(day.date.clone()),
                brand_key: Some(b.key),
                brand: Some(b.name),
                domain: b.domain,
                is_owned: Some(false),
                visibility_score: mention_rate(&cur),
                share_of_voice: share_of_voice(cur.mentioned, all - cur.mentioned),
                average_position: average_position(&cur),
                mentions: Some(cur.mentioned as i64),
                answer_count: Some(day.own.answers),
                ..MetricRow::default()
            });
        }
    }
    Ok(rows)
}
