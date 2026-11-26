//! Turns events into inbox notifications and signed webhook deliveries, and sends the
//! deliveries with retries. One loop in the API process; several API processes are safe
//! because every batch is claimed with `FOR UPDATE SKIP LOCKED`.

use crate::notifications::{NotificationRule, parse_clock, rule_for};
use chrono::{DateTime, Duration as Span, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::{PgPool, Postgres, Transaction};
use std::{str::FromStr, time::Duration};
use uuid::Uuid;

/// A delivery is tried this many times, then marked `failed`.
const MAX_ATTEMPTS: i32 = 6;
/// Seconds to wait after attempt 1, 2, 3, 4, 5.
const BACKOFF_SECS: [i64; 5] = [30, 120, 600, 3600, 21_600];
const BATCH: i64 = 100;
/// An inbox batch stops growing past this many entries.
const MAX_BATCH_ITEMS: i64 = 50;

type Tx<'a> = Transaction<'a, Postgres>;

/// HMAC-SHA256 as lowercase hex.
pub fn hmac_hex(key: &str, message: &str) -> String {
    // Any key length is valid for HMAC, so `new_from_slice` cannot fail here.
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).unwrap_or_else(|_| unreachable!());
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// The `X-OpenJam-Signature` value: `t=<unix seconds>,v1=<hmac of "<t>.<body>">`.
pub fn signature(secret: &str, timestamp: i64, body: &str) -> String {
    format!(
        "t={timestamp},v1={}",
        hmac_hex(secret, &format!("{timestamp}.{body}"))
    )
}

/// The JSON a webhook receives and the SSE stream sends.
pub fn envelope(
    id: i64,
    event_type: &str,
    category: &str,
    created_at: DateTime<Utc>,
    workspace_id: Uuid,
    data: &Value,
) -> Value {
    json!({
        "id": id,
        "type": event_type,
        "category": category,
        "createdAt": created_at,
        "workspaceId": workspace_id,
        "data": data,
    })
}

/// When the quiet window that holds `now` ends, or `None` outside the window.
pub fn quiet_until(
    now: DateTime<Utc>,
    start: NaiveTime,
    end: NaiveTime,
    tz: Tz,
) -> Option<DateTime<Utc>> {
    let local = now.with_timezone(&tz);
    let time = local.time();
    let wraps = start > end;
    let inside = if wraps {
        time >= start || time < end
    } else {
        time >= start && time < end
    };
    if !inside {
        return None;
    }
    let date = if wraps && time >= start {
        local.date_naive().succ_opt()?
    } else {
        local.date_naive()
    };
    let ends = tz.from_local_datetime(&date.and_time(end));
    ends.earliest()
        .or(ends.latest())
        .map(|t| t.with_timezone(&Utc))
}

fn deliver_after(rule: &NotificationRule, now: DateTime<Utc>) -> DateTime<Utc> {
    let Some(q) = &rule.quiet_hours else {
        return now;
    };
    let (Some(start), Some(end), Ok(tz)) = (
        parse_clock(&q.start),
        parse_clock(&q.end),
        Tz::from_str(&q.timezone),
    ) else {
        return now;
    };
    quiet_until(now, start, end, tz).unwrap_or(now)
}

/// Seconds to wait before the next try, after `attempts` tries. `None`: give up.
pub fn backoff(attempts: i32) -> Option<i64> {
    if attempts >= MAX_ATTEMPTS {
        return None;
    }
    BACKOFF_SECS
        .get(usize::try_from(attempts - 1).unwrap_or(0))
        .copied()
}

#[derive(sqlx::FromRow)]
struct EventRow {
    id: i64,
    workspace_id: Uuid,
    #[sqlx(rename = "type")]
    event_type: String,
    category: String,
    payload: Value,
    created_at: DateTime<Utc>,
}

/// Fan one batch of new events out to the inbox and the webhook queue. Returns how many.
pub async fn fan_out(pool: &PgPool) -> sqlx::Result<usize> {
    let mut tx = pool.begin().await?;
    let events: Vec<EventRow> = sqlx::query_as(
        "SELECT id, workspace_id, type, category, payload, created_at FROM events
         WHERE fanned_out_at IS NULL ORDER BY id LIMIT $1 FOR UPDATE SKIP LOCKED",
    )
    .bind(BATCH)
    .fetch_all(&mut *tx)
    .await?;
    for event in &events {
        let rule = rule_for(pool, event.workspace_id, &event.category).await?;
        if rule.is_active {
            let body = envelope(
                event.id,
                &event.event_type,
                &event.category,
                event.created_at,
                event.workspace_id,
                &event.payload,
            );
            if rule.channels.iter().any(|c| c == "webhook") {
                sqlx::query(
                    "INSERT INTO webhook_deliveries (webhook_id, event, payload, event_id, next_attempt_at)
                     SELECT id, $2, $3, $4, $5 FROM webhooks WHERE workspace_id = $1 AND enabled",
                )
                .bind(event.workspace_id)
                .bind(&event.event_type)
                .bind(&body)
                .bind(event.id)
                .bind(deliver_after(&rule, Utc::now()))
                .execute(&mut *tx)
                .await?;
            }
            if rule.channels.iter().any(|c| c == "chat") {
                notify_members(&mut tx, event, &rule, &body).await?;
            }
        }
        sqlx::query("UPDATE events SET fanned_out_at = now() WHERE id = $1")
            .bind(event.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(events.len())
}

async fn notify_members(
    tx: &mut Tx<'_>,
    event: &EventRow,
    rule: &NotificationRule,
    body: &Value,
) -> sqlx::Result<()> {
    if !rule.batching_enabled {
        sqlx::query(
            "INSERT INTO notifications (user_id, workspace_id, kind, category, body)
             SELECT user_id, workspace_id, $2, $3, $4 FROM memberships WHERE workspace_id = $1",
        )
        .bind(event.workspace_id)
        .bind(&event.event_type)
        .bind(&event.category)
        .bind(body)
        .execute(&mut **tx)
        .await?;
        return Ok(());
    }
    // Batching: add to the newest unread batch of this category if its window is open.
    let kind = format!("batch.{}", event.category);
    let members: Vec<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM memberships WHERE workspace_id = $1")
            .bind(event.workspace_id)
            .fetch_all(&mut **tx)
            .await?;
    for user_id in members {
        let merged = sqlx::query(
            "UPDATE notifications SET body = jsonb_build_object(
                 'count', (body->>'count')::int + 1,
                 'events', CASE WHEN (body->>'count')::int < $5 THEN body->'events' || jsonb_build_array($6::jsonb)
                                ELSE body->'events' END)
             WHERE id = (SELECT id FROM notifications
                         WHERE user_id = $1 AND workspace_id = $2 AND kind = $3 AND read_at IS NULL
                           AND created_at > now() - make_interval(mins => $4)
                         ORDER BY created_at DESC LIMIT 1)",
        )
        .bind(user_id)
        .bind(event.workspace_id)
        .bind(&kind)
        .bind(rule.batch_window_minutes)
        .bind(MAX_BATCH_ITEMS)
        .bind(body)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        if merged == 0 {
            sqlx::query(
                "INSERT INTO notifications (user_id, workspace_id, kind, category, body)
                 VALUES ($1, $2, $3, $4, jsonb_build_object('count', 1, 'events', jsonb_build_array($5::jsonb)))",
            )
            .bind(user_id)
            .bind(event.workspace_id)
            .bind(&kind)
            .bind(&event.category)
            .bind(body)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Due {
    id: Uuid,
    event: String,
    payload: Value,
    attempts: i32,
    url: String,
    secret: String,
}

/// Send one batch of due deliveries. Returns how many were tried.
pub async fn deliver(pool: &PgPool, client: &reqwest::Client) -> sqlx::Result<usize> {
    // Claim by leasing: push `next_attempt_at` out, so a crash retries after the lease.
    let due: Vec<Due> = sqlx::query_as(
        "UPDATE webhook_deliveries d SET attempts = d.attempts + 1, next_attempt_at = now() + interval '2 minutes'
         FROM webhooks w
         WHERE w.id = d.webhook_id AND d.id IN (
             SELECT d2.id FROM webhook_deliveries d2 JOIN webhooks w2 ON w2.id = d2.webhook_id
             WHERE d2.status = 'queued' AND d2.next_attempt_at <= now() AND w2.enabled
             ORDER BY d2.next_attempt_at LIMIT 20 FOR UPDATE OF d2 SKIP LOCKED)
         RETURNING d.id, d.event, d.payload, d.attempts, w.url, w.secret",
    )
    .fetch_all(pool)
    .await?;
    let count = due.len();
    let sends = due.into_iter().map(|d| async move {
        let result = send(client, &d).await;
        record(pool, &d, result).await
    });
    for outcome in futures_util::future::join_all(sends).await {
        outcome?;
    }
    Ok(count)
}

async fn send(client: &reqwest::Client, d: &Due) -> Result<u16, (Option<u16>, String)> {
    let body = d.payload.to_string();
    let timestamp = Utc::now().timestamp();
    let response = client
        .post(&d.url)
        .header("content-type", "application/json")
        .header("user-agent", "openJam-Webhooks/1")
        .header("x-openjam-event", &d.event)
        .header("x-openjam-delivery", d.id.to_string())
        .header("x-openjam-timestamp", timestamp.to_string())
        .header(
            "x-openjam-signature",
            signature(&d.secret, timestamp, &body),
        )
        .body(body)
        .send()
        .await
        .map_err(|e| (None, e.without_url().to_string()))?;
    let code = response.status().as_u16();
    if response.status().is_success() {
        Ok(code)
    } else {
        Err((Some(code), format!("the endpoint answered HTTP {code}")))
    }
}

async fn record(
    pool: &PgPool,
    d: &Due,
    result: Result<u16, (Option<u16>, String)>,
) -> sqlx::Result<()> {
    match result {
        Ok(code) => {
            sqlx::query(
                "UPDATE webhook_deliveries SET status = 'delivered', response_code = $2, last_error = NULL WHERE id = $1",
            )
            .bind(d.id)
            .bind(i32::from(code))
            .execute(pool)
            .await?;
        }
        Err((code, error)) => match backoff(d.attempts) {
            Some(secs) => {
                sqlx::query(
                    "UPDATE webhook_deliveries SET response_code = $2, last_error = $3, next_attempt_at = $4 WHERE id = $1",
                )
                .bind(d.id)
                .bind(code.map(i32::from))
                .bind(&error)
                .bind(Utc::now() + Span::seconds(secs))
                .execute(pool)
                .await?;
            }
            None => {
                sqlx::query(
                    "UPDATE webhook_deliveries SET status = 'failed', response_code = $2, last_error = $3 WHERE id = $1",
                )
                .bind(d.id)
                .bind(code.map(i32::from))
                .bind(&error)
                .execute(pool)
                .await?;
            }
        },
    }
    Ok(())
}

/// Drop rows nobody reads any more.
pub async fn prune(pool: &PgPool) -> sqlx::Result<()> {
    for sql in [
        "DELETE FROM events WHERE created_at < now() - interval '30 days' AND fanned_out_at IS NOT NULL",
        "DELETE FROM webhook_deliveries WHERE created_at < now() - interval '30 days' AND status <> 'queued'",
        "DELETE FROM notifications WHERE created_at < now() - interval '90 days'",
        "DELETE FROM oauth_codes WHERE expires_at < now() - interval '1 day'",
        "DELETE FROM oauth_tokens WHERE expires_at < now() - interval '60 days'",
        "DELETE FROM agent_connect_sessions WHERE expires_at < now() - interval '1 day'",
    ] {
        sqlx::query(sql).execute(pool).await?;
    }
    Ok(())
}

/// Start the loop. It runs for the life of the process.
pub fn spawn(pool: PgPool) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        // A redirect could carry a signed body to a host the owner never chose.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default();
    tokio::spawn(async move {
        let mut tick: u64 = 0;
        loop {
            if let Err(e) = fan_out(&pool).await {
                tracing::error!(error = %e, "event fan-out failed");
            }
            if let Err(e) = deliver(&pool, &client).await {
                tracing::error!(error = %e, "webhook delivery failed");
            }
            if tick.is_multiple_of(600)
                && let Err(e) = prune(&pool).await
            {
                tracing::error!(error = %e, "prune failed");
            }
            tick += 1;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 26, h, m, 0).unwrap()
    }

    fn clock(s: &str) -> NaiveTime {
        parse_clock(s).unwrap()
    }

    #[test]
    fn hmac_matches_rfc_4231_case_2() {
        assert_eq!(
            hmac_hex("Jefe", "what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn signature_covers_timestamp_and_body() {
        let a = signature("s", 100, "{}");
        assert!(a.starts_with("t=100,v1="));
        assert_ne!(a, signature("s", 101, "{}"));
        assert_ne!(a, signature("s", 100, "{ }"));
    }

    #[test]
    fn quiet_window_across_midnight() {
        let (start, end) = (clock("22:00"), clock("07:00"));
        assert_eq!(
            quiet_until(at(23, 30), start, end, Tz::UTC),
            Some(Utc.with_ymd_and_hms(2026, 11, 27, 7, 0, 0).unwrap())
        );
        assert_eq!(quiet_until(at(3, 0), start, end, Tz::UTC), Some(at(7, 0)));
        assert_eq!(quiet_until(at(12, 0), start, end, Tz::UTC), None);
        assert_eq!(quiet_until(at(7, 0), start, end, Tz::UTC), None);
    }

    #[test]
    fn quiet_window_uses_its_own_time_zone() {
        // 23:30 UTC is 00:30 in Berlin (UTC+1 in November): inside 22:00 to 07:00 there.
        let berlin: Tz = "Europe/Berlin".parse().unwrap();
        assert_eq!(
            quiet_until(at(23, 30), clock("22:00"), clock("07:00"), berlin),
            Some(at(6, 0)).map(|t| t + Span::days(1))
        );
        // The same instant is 15:30 in Los Angeles: outside.
        let la: Tz = "America/Los_Angeles".parse().unwrap();
        assert_eq!(
            quiet_until(at(23, 30), clock("22:00"), clock("07:00"), la),
            None
        );
    }

    #[test]
    fn retries_back_off_then_stop() {
        assert_eq!(backoff(1), Some(30));
        assert_eq!(backoff(5), Some(21_600));
        assert_eq!(backoff(6), None);
    }
}
