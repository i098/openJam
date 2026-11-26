//! Shared HTTP helpers: one client shape and one mapping from provider failures to `EngineError`.

use crate::EngineError;
use chrono::{DateTime, Utc};
use std::time::Duration;

/// Per-attempt limit for a provider call.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(CALL_TIMEOUT)
        .build()
        .expect("the default HTTP client builds")
}

pub fn send_error(e: reqwest::Error) -> EngineError {
    if e.is_timeout() {
        EngineError::Timeout
    } else {
        // `without_url`: the URL can hold a query key (Gemini key calls use a header, but be safe).
        EngineError::Unavailable(format!("request failed: {}", e.without_url()))
    }
}

/// `Retry-After` as seconds from now. A date value is ignored.
pub fn retry_at(header: Option<&str>) -> Option<DateTime<Utc>> {
    let secs: i64 = header?.trim().parse().ok()?;
    Some(Utc::now() + chrono::Duration::seconds(secs.clamp(0, 86_400)))
}

/// Map a non-success reply. `body` is the response text.
pub fn map_status(status: u16, retry_after: Option<&str>, body: &str) -> EngineError {
    let has = |needle: &str| body.contains(needle);
    match status {
        429 => EngineError::RateLimited {
            retry_at: retry_at(retry_after),
            usage_limit: has("usage_limit_exceeded") || has("insufficient_quota"),
        },
        401 => EngineError::Unauthorized,
        403 if has("user_not_eligible") => EngineError::NotEligible,
        403 if has("route_not_supported") => EngineError::SearchUnsupported,
        403 => EngineError::Unauthorized,
        400 if has("unsupported_capability") => EngineError::SearchUnsupported,
        _ => EngineError::Unavailable(format!(
            "provider answered {status}: {}",
            body.chars().take(200).collect::<String>()
        )),
    }
}

/// POST JSON and return the parsed body, or the mapped failure.
pub async fn post_json(
    http: &reqwest::Client,
    url: &str,
    headers: &[(&str, String)],
    body: &serde_json::Value,
) -> Result<serde_json::Value, EngineError> {
    let mut req = http.post(url).json(body);
    for (name, value) in headers {
        req = req.header(*name, value);
    }
    let resp = req.send().await.map_err(send_error)?;
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let text = resp.text().await.map_err(send_error)?;
    if status >= 400 {
        return Err(map_status(status, retry_after.as_deref(), &text));
    }
    serde_json::from_str(&text)
        .map_err(|e| EngineError::Unavailable(format!("bad JSON reply: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_to_the_design_events() {
        assert!(matches!(
            map_status(
                429,
                Some("30"),
                r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}"#
            ),
            EngineError::RateLimited {
                usage_limit: true,
                retry_at: Some(_)
            }
        ));
        assert!(matches!(
            map_status(429, None, "slow down"),
            EngineError::RateLimited {
                usage_limit: false,
                retry_at: None
            }
        ));
        assert!(matches!(
            map_status(403, None, "subscription_sharing_user_not_eligible"),
            EngineError::NotEligible
        ));
        assert!(matches!(
            map_status(400, None, "subscription_sharing_unsupported_capability"),
            EngineError::SearchUnsupported
        ));
        assert!(matches!(
            map_status(401, None, ""),
            EngineError::Unauthorized
        ));
        assert!(matches!(
            map_status(503, None, "down"),
            EngineError::Unavailable(_)
        ));
    }
}
