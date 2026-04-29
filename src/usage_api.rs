use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use chrono::{DateTime, Utc};
use objc2::msg_send;
use objc2_foundation::{
    NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURL,
    NSURLRequestCachePolicy, NSURLResponse, NSURLSession,
};
use serde::Deserialize;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA_HEADER: &str = "oauth-2025-04-20";
const USER_AGENT: &str = "claude-code/2.1.0";

#[derive(Debug, thiserror::Error)]
pub enum UsageError {
    #[error("network error: {0}")]
    Network(String),
    #[error("unauthorized (HTTP 401) — token expired or revoked; run `claude` to refresh")]
    Unauthorized,
    #[error("forbidden (HTTP 403) — token likely missing 'user:profile' scope: {0}")]
    Forbidden(String),
    #[error("server returned HTTP {0}: {1}")]
    Status(u16, String),
    #[error("JSON parse failed: {0}")]
    Json(#[from] serde_json::Error),
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
    let (tx, rx) = mpsc::channel::<Result<String, UsageError>>();

    let url = unsafe {
        NSURL::URLWithString(&NSString::from_str(USAGE_URL))
    }
    .ok_or_else(|| UsageError::Network("invalid URL".into()))?;

    let request = unsafe { NSMutableURLRequest::new() };
    unsafe {
        request.setURL(Some(&url));
        request.setTimeoutInterval(30.0);
        request.setCachePolicy(NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData);
        let auth = format!("Bearer {access_token}");
        for (k, v) in [
            ("Authorization", auth.as_str()),
            ("anthropic-beta", BETA_HEADER),
            ("accept", "application/json"),
            ("content-type", "application/json"),
            ("user-agent", USER_AGENT),
        ] {
            request.setValue_forHTTPHeaderField(
                Some(&NSString::from_str(v)),
                &NSString::from_str(k),
            );
        }
    }

    let block = RcBlock::new(
        move |data: *mut NSData, response: *mut NSURLResponse, error: *mut NSError| {
            let result = unsafe {
                if !error.is_null() {
                    let desc = (*error).localizedDescription();
                    Err(UsageError::Network(desc.to_string()))
                } else if data.is_null() {
                    Err(UsageError::Network("empty response".into()))
                } else {
                    let status: isize = if response.is_null() {
                        200
                    } else {
                        let http = &*(response as *const NSHTTPURLResponse);
                        http.statusCode()
                    };
                    let bytes: *const u8 = msg_send![&*data, bytes];
                    let len = (*data).length() as usize;
                    let body =
                        std::str::from_utf8(std::slice::from_raw_parts(bytes, len))
                            .unwrap_or("")
                            .to_string();
                    match status as u16 {
                        200 => Ok(body),
                        401 => Err(UsageError::Unauthorized),
                        403 => Err(UsageError::Forbidden(snippet(&body, 300))),
                        s => Err(UsageError::Status(s, snippet(&body, 300))),
                    }
                }
            };
            tx.send(result).ok();
        },
    );

    unsafe {
        let session = NSURLSession::sharedSession();
        let task = session.dataTaskWithRequest_completionHandler(&request, &block);
        task.resume();
    }

    match rx.recv_timeout(Duration::from_secs(35)) {
        Ok(Ok(body)) => parse_body(&body),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(UsageError::Network("request timed out".into())),
    }
}

fn parse_body(body: &str) -> Result<UsageSnapshot, UsageError> {
    let payload: UsageResponse = serde_json::from_str(body)?;
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
