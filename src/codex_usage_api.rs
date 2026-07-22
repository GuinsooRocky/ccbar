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

use crate::usage_api::WindowState;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const USER_AGENT: &str = concat!("ccbar/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, thiserror::Error)]
pub enum CodexUsageError {
    #[error("network error: {0}")]
    Network(String),
    #[error("unauthorized — token expired or revoked; run `codex` to refresh it")]
    Unauthorized,
    #[error("server returned HTTP {0}: {1}")]
    Status(u16, String),
    #[error("JSON parse failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("response contains no Codex rate-limit windows")]
    MissingWindows,
}

#[derive(Debug, Clone)]
pub struct CodexUsageSnapshot {
    pub windows: Vec<CodexWindow>,
    pub plan: Option<String>,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CodexWindow {
    pub label: String,
    pub state: WindowState,
}

pub fn fetch_usage(
    access_token: &str,
    account_id: Option<&str>,
) -> Result<CodexUsageSnapshot, CodexUsageError> {
    let (tx, rx) = mpsc::channel::<Result<String, CodexUsageError>>();

    let url = NSURL::URLWithString(&NSString::from_str(USAGE_URL))
        .ok_or_else(|| CodexUsageError::Network("invalid URL".into()))?;
    let request = NSMutableURLRequest::new();
    request.setURL(Some(&url));
    request.setTimeoutInterval(30.0);
    request.setCachePolicy(NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData);
    let auth = format!("Bearer {access_token}");
    for (key, value) in [
        ("Authorization", auth.as_str()),
        ("Accept", "application/json"),
        ("User-Agent", USER_AGENT),
    ] {
        request.setValue_forHTTPHeaderField(
            Some(&NSString::from_str(value)),
            &NSString::from_str(key),
        );
    }
    if let Some(account_id) = account_id.filter(|value| !value.is_empty()) {
        request.setValue_forHTTPHeaderField(
            Some(&NSString::from_str(account_id)),
            &NSString::from_str("ChatGPT-Account-Id"),
        );
    }

    let block = RcBlock::new(
        move |data: *mut NSData, response: *mut NSURLResponse, error: *mut NSError| {
            let result = unsafe {
                if !error.is_null() {
                    Err(CodexUsageError::Network(
                        (*error).localizedDescription().to_string(),
                    ))
                } else if data.is_null() {
                    Err(CodexUsageError::Network("empty response".into()))
                } else {
                    let status: isize = if response.is_null() {
                        200
                    } else {
                        let http = &*(response as *const NSHTTPURLResponse);
                        http.statusCode()
                    };
                    let bytes: *const u8 = msg_send![&*data, bytes];
                    let len = (*data).length() as usize;
                    let body = std::str::from_utf8(std::slice::from_raw_parts(bytes, len))
                        .unwrap_or("")
                        .to_owned();
                    match status as u16 {
                        200 => Ok(body),
                        401 | 403 => Err(CodexUsageError::Unauthorized),
                        code => Err(CodexUsageError::Status(code, snippet(&body, 300))),
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
        Ok(Err(error)) => Err(error),
        Err(_) => Err(CodexUsageError::Network("request timed out".into())),
    }
}

fn parse_body(body: &str) -> Result<CodexUsageSnapshot, CodexUsageError> {
    let response: UsageResponse = serde_json::from_str(body)?;
    let mut windows = rate_limit_windows(response.rate_limit.as_ref(), None);

    for additional in response.additional_rate_limits.unwrap_or_default() {
        let Some(label) = additional
            .limit_name
            .as_deref()
            .or(additional.metered_feature.as_deref())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        windows.extend(rate_limit_windows(
            additional.rate_limit.as_ref(),
            Some(label),
        ));
    }

    if windows.is_empty() {
        return Err(CodexUsageError::MissingWindows);
    }

    Ok(CodexUsageSnapshot {
        windows,
        plan: response.plan_type,
        fetched_at: Utc::now(),
    })
}

fn rate_limit_windows(
    rate_limit: Option<&RateLimitPayload>,
    scope: Option<&str>,
) -> Vec<CodexWindow> {
    let Some(rate_limit) = rate_limit else {
        return Vec::new();
    };
    let payloads = [
        rate_limit.primary_window.as_ref(),
        rate_limit.secondary_window.as_ref(),
    ];
    let valid_count = payloads
        .iter()
        .flatten()
        .filter(|window| window.is_valid())
        .count();

    payloads
        .into_iter()
        .flatten()
        .filter_map(|window| {
            let state = window.to_state()?;
            let duration_label = duration_label(window.limit_window_seconds?);
            let label = match (scope, valid_count) {
                (Some(scope), 1) => compact_limit_label(scope),
                (Some(scope), _) => format!("{} · {duration_label}", compact_limit_label(scope)),
                (None, _) => duration_label,
            };
            Some(CodexWindow { label, state })
        })
        .collect()
}

fn compact_limit_label(label: &str) -> String {
    label
        .rsplit_once("-Codex-")
        .map(|(_, model)| model.replace('-', " "))
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| label.to_owned())
}

fn duration_label(seconds: i64) -> String {
    match seconds {
        18_000 => "Session".into(),
        604_800 => "Weekly".into(),
        seconds if seconds > 0 && seconds % 86_400 == 0 => format!("{}d window", seconds / 86_400),
        seconds if seconds > 0 && seconds % 3_600 == 0 => format!("{}h window", seconds / 3_600),
        seconds if seconds > 0 && seconds % 60 == 0 => format!("{}m window", seconds / 60),
        seconds => format!("{seconds}s window"),
    }
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    plan_type: Option<String>,
    rate_limit: Option<RateLimitPayload>,
    additional_rate_limits: Option<Vec<AdditionalRateLimitPayload>>,
}

#[derive(Debug, Deserialize)]
struct AdditionalRateLimitPayload {
    limit_name: Option<String>,
    metered_feature: Option<String>,
    rate_limit: Option<RateLimitPayload>,
}

#[derive(Debug, Deserialize)]
struct RateLimitPayload {
    primary_window: Option<WindowPayload>,
    secondary_window: Option<WindowPayload>,
}

#[derive(Debug, Deserialize)]
struct WindowPayload {
    used_percent: Option<f64>,
    reset_at: Option<i64>,
    limit_window_seconds: Option<i64>,
}

impl WindowPayload {
    fn is_valid(&self) -> bool {
        self.used_percent.is_some() && self.limit_window_seconds.is_some()
    }

    fn to_state(&self) -> Option<WindowState> {
        Some(WindowState {
            fraction_used: self.used_percent? / 100.0,
            resets_at: self
                .reset_at
                .and_then(|seconds| DateTime::from_timestamp(seconds, 0)),
        })
    }
}

fn snippet(body: &str, max: usize) -> String {
    let cleaned = body.trim().replace('\n', " ");
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        format!("{}…", cleaned.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_real_weekly_only_shape_and_additional_limit() {
        let body = r#"{
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 3,
                    "limit_window_seconds": 604800,
                    "reset_at": 1785258196
                },
                "secondary_window": null
            },
            "additional_rate_limits": [{
                "limit_name": "GPT-5.3-Codex-Spark",
                "metered_feature": "codex_bengalfox",
                "rate_limit": {
                    "primary_window": {
                        "used_percent": 0,
                        "limit_window_seconds": 604800,
                        "reset_at": 1785299799
                    }
                }
            }]
        }"#;

        let snapshot = parse_body(body).expect("parse");
        assert_eq!(snapshot.plan.as_deref(), Some("pro"));
        assert_eq!(snapshot.windows.len(), 2);
        assert_eq!(snapshot.windows[0].label, "Weekly");
        assert_eq!(snapshot.windows[0].state.percent_left(), 97);
        assert_eq!(snapshot.windows[1].label, "Spark");
        assert_eq!(snapshot.windows[1].state.percent_left(), 100);
    }

    #[test]
    fn labels_primary_and_secondary_by_actual_duration() {
        let body = r#"{
            "rate_limit": {
                "primary_window": {
                    "used_percent": 22,
                    "limit_window_seconds": 18000,
                    "reset_at": 1766948068
                },
                "secondary_window": {
                    "used_percent": 43,
                    "limit_window_seconds": 604800,
                    "reset_at": 1767407914
                }
            }
        }"#;

        let snapshot = parse_body(body).expect("parse");
        assert_eq!(snapshot.windows[0].label, "Session");
        assert_eq!(snapshot.windows[1].label, "Weekly");
        assert_eq!(snapshot.windows[0].state.percent_left(), 78);
        assert_eq!(snapshot.windows[1].state.percent_left(), 57);
    }

    #[test]
    fn rejects_payload_without_usable_windows() {
        assert!(matches!(
            parse_body(
                r#"{
                    "plan_type": "plus",
                    "rate_limit": null,
                    "additional_rate_limits": null
                }"#
            ),
            Err(CodexUsageError::MissingWindows)
        ));
    }
}
