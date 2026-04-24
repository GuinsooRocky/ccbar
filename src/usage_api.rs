//! Anthropic OAuth usage API client.
//!
//! Endpoint: `GET https://api.anthropic.com/api/oauth/usage`
//! Response shape (fields we care about):
//!
//! ```json
//! {
//!   "five_hour":        { "utilization": 0.59, "resets_at": "2026-04-24T02:00:00Z" },
//!   "seven_day":        { "utilization": 0.82, "resets_at": "..." },
//!   "seven_day_sonnet": { "utilization": 0.09, "resets_at": "..." },
//!   "seven_day_opus":   { ... }
//! }
//! ```
//!
//! The beta header `anthropic-beta: oauth-2025-04-20` is currently required.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA_HEADER: &str = "oauth-2025-04-20";
const USER_AGENT: &str = "claude-code/2.1.0";
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum UsageError {
    #[error("http transport failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("unauthorized (HTTP 401) — token expired or revoked; run `claude` to refresh")]
    Unauthorized,
    #[error("forbidden (HTTP 403) — token likely missing 'user:profile' scope: {0}")]
    Forbidden(String),
    #[error("server returned HTTP {0}: {1}")]
    Status(u16, String),
    #[error("response missing 'five_hour' window")]
    MissingSession,
}

#[derive(Debug, Clone)]
pub struct UsageSnapshot {
    pub session: WindowState,
    pub weekly: Option<WindowState>,
    pub sonnet: Option<WindowState>,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct WindowState {
    /// 0.0 = unused, 1.0 = fully consumed.
    pub fraction_used: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

impl WindowState {
    pub fn percent_left(&self) -> i32 {
        let raw = (1.0 - self.fraction_used.clamp(0.0, 1.0)) * 100.0;
        raw.round() as i32
    }

    pub fn reset_label(&self) -> String {
        match self.resets_at {
            Some(at) => format_countdown(at, Utc::now()),
            None => String::new(),
        }
    }
}

pub fn fetch_usage(access_token: &str) -> Result<UsageSnapshot, UsageError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .build()?;

    let response = client
        .get(USAGE_URL)
        .bearer_auth(access_token)
        .header("anthropic-beta", BETA_HEADER)
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .header("user-agent", USER_AGENT)
        .send()?;

    let status = response.status();
    let status_code = status.as_u16();
    match status_code {
        200 => {}
        401 => return Err(UsageError::Unauthorized),
        403 => {
            let body = response.text().unwrap_or_default();
            return Err(UsageError::Forbidden(snippet(&body, 300)));
        }
        _ => {
            let body = response.text().unwrap_or_default();
            return Err(UsageError::Status(status_code, snippet(&body, 300)));
        }
    }

    let payload: UsageResponse = response.json()?;
    let session = payload
        .five_hour
        .and_then(WindowPayload::into_state)
        .ok_or(UsageError::MissingSession)?;
    let weekly = payload.seven_day.and_then(WindowPayload::into_state);
    let sonnet = payload
        .seven_day_sonnet
        .and_then(WindowPayload::into_state)
        .or_else(|| payload.seven_day_opus.and_then(WindowPayload::into_state));

    Ok(UsageSnapshot {
        session,
        weekly,
        sonnet,
        fetched_at: Utc::now(),
    })
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    five_hour: Option<WindowPayload>,
    seven_day: Option<WindowPayload>,
    seven_day_sonnet: Option<WindowPayload>,
    seven_day_opus: Option<WindowPayload>,
}

#[derive(Debug, Deserialize)]
struct WindowPayload {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

impl WindowPayload {
    fn into_state(self) -> Option<WindowState> {
        // API returns `utilization` as a 0–100 percent, not a 0–1 fraction.
        let fraction = self.utilization? / 100.0;
        let resets_at = self
            .resets_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        Some(WindowState {
            fraction_used: fraction,
            resets_at,
        })
    }
}

fn format_countdown(target: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let delta = target.signed_duration_since(now);
    let total = delta.num_seconds();
    if total <= 0 {
        return "now".into();
    }
    let days = total / 86_400;
    let hours = (total % 86_400) / 3_600;
    let mins = (total % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

fn snippet(body: &str, max: usize) -> String {
    let cleaned = body.trim().replace('\n', " ");
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        let truncated: String = cleaned.chars().take(max).collect();
        format!("{truncated}…")
    }
}
