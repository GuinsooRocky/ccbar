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
    pub scoped: Option<ScopedWindow>,
    pub fetched_at: DateTime<Utc>,
}

/// The weekly quota carved out for the current premium model. Anthropic renames
/// that model every few months, so the label travels with the data instead of
/// being compiled in.
#[derive(Debug, Clone)]
pub struct ScopedWindow {
    pub label: String,
    pub state: WindowState,
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
    let UsageResponse {
        five_hour,
        seven_day,
        seven_day_sonnet,
        seven_day_opus,
        limits,
    } = serde_json::from_str::<UsageResponse>(body)?;

    // `limits` is the current, self-describing shape: every entry names its own
    // window and the model row carries `scope.model.display_name`. The old
    // `seven_day_sonnet` / `seven_day_opus` keys went null when Anthropic moved
    // to per-model scopes, so they are only a fallback for older responses.
    let session = limits
        .iter()
        .find(|l| l.kind.as_deref() == Some("session"))
        .and_then(LimitPayload::to_state)
        .or_else(|| five_hour.and_then(WindowPayload::into_state))
        .ok_or(UsageError::MissingSession)?;

    let weekly = limits
        .iter()
        .find(|l| l.kind.as_deref() == Some("weekly_all"))
        .and_then(LimitPayload::to_state)
        .or_else(|| seven_day.and_then(WindowPayload::into_state));

    let scoped = limits
        .iter()
        .find_map(|l| {
            Some(ScopedWindow {
                label: l.model_name()?.to_owned(),
                state: l.to_state()?,
            })
        })
        .or_else(|| {
            let (label, payload) = match (seven_day_sonnet, seven_day_opus) {
                (Some(p), _) => ("Sonnet", p),
                (None, Some(p)) => ("Opus", p),
                (None, None) => return None,
            };
            Some(ScopedWindow {
                label: label.to_owned(),
                state: payload.into_state()?,
            })
        });

    Ok(UsageSnapshot {
        session,
        weekly,
        scoped,
        fetched_at: Utc::now(),
    })
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    five_hour: Option<WindowPayload>,
    seven_day: Option<WindowPayload>,
    seven_day_sonnet: Option<WindowPayload>,
    seven_day_opus: Option<WindowPayload>,
    #[serde(default)]
    limits: Vec<LimitPayload>,
}

#[derive(Debug, Deserialize)]
struct WindowPayload {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

impl WindowPayload {
    fn into_state(self) -> Option<WindowState> {
        let fraction = self.utilization? / 100.0;
        Some(WindowState {
            fraction_used: fraction,
            resets_at: parse_reset(self.resets_at.as_deref()),
        })
    }
}

#[derive(Debug, Deserialize)]
struct LimitPayload {
    kind: Option<String>,
    percent: Option<f64>,
    resets_at: Option<String>,
    scope: Option<ScopePayload>,
}

#[derive(Debug, Deserialize)]
struct ScopePayload {
    model: Option<ModelPayload>,
}

#[derive(Debug, Deserialize)]
struct ModelPayload {
    display_name: Option<String>,
}

impl LimitPayload {
    fn to_state(&self) -> Option<WindowState> {
        Some(WindowState {
            fraction_used: self.percent? / 100.0,
            resets_at: parse_reset(self.resets_at.as_deref()),
        })
    }

    fn model_name(&self) -> Option<&str> {
        self.scope
            .as_ref()?
            .model
            .as_ref()?
            .display_name
            .as_deref()
            .filter(|s| !s.is_empty())
    }
}

fn parse_reset(raw: Option<&str>) -> Option<DateTime<Utc>> {
    raw.and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from the live endpoint on 2026-07-13: the per-model top-level
    /// keys are all null and the model name lives in `limits[].scope`.
    const CURRENT: &str = r#"{
        "five_hour":        { "utilization": 98.0, "resets_at": "2026-07-13T08:50:00.002962+00:00" },
        "seven_day":        { "utilization": 22.0, "resets_at": "2026-07-18T22:00:00.002988+00:00" },
        "seven_day_opus":   null,
        "seven_day_sonnet": null,
        "tangelo":          null,
        "limits": [
            { "kind": "session",       "percent": 98, "resets_at": "2026-07-13T08:50:00.002962+00:00", "scope": null },
            { "kind": "weekly_all",    "percent": 22, "resets_at": "2026-07-18T22:00:00.002988+00:00", "scope": null },
            { "kind": "weekly_scoped", "percent": 15, "resets_at": "2026-07-18T22:00:00.003411+00:00",
              "scope": { "model": { "id": null, "display_name": "Fable" }, "surface": null } }
        ]
    }"#;

    /// The pre-`limits` shape, still served to some accounts.
    const LEGACY: &str = r#"{
        "five_hour":        { "utilization": 40.0, "resets_at": null },
        "seven_day":        { "utilization": 30.0, "resets_at": null },
        "seven_day_sonnet": { "utilization": 9.0,  "resets_at": null }
    }"#;

    #[test]
    fn reads_model_name_from_limits_scope() {
        let snap = parse_body(CURRENT).expect("parse");
        assert_eq!(snap.session.percent_left(), 2);
        assert_eq!(snap.weekly.expect("weekly").percent_left(), 78);

        let scoped = snap.scoped.expect("scoped window");
        assert_eq!(scoped.label, "Fable");
        assert_eq!(scoped.state.percent_left(), 85);
    }

    #[test]
    fn falls_back_to_legacy_windows_without_limits() {
        let snap = parse_body(LEGACY).expect("parse");
        assert_eq!(snap.session.percent_left(), 60);
        assert_eq!(snap.weekly.expect("weekly").percent_left(), 70);

        let scoped = snap.scoped.expect("scoped window");
        assert_eq!(scoped.label, "Sonnet");
        assert_eq!(scoped.state.percent_left(), 91);
    }

    #[test]
    fn omits_scoped_window_when_absent_everywhere() {
        let body = r#"{ "five_hour": { "utilization": 10.0 }, "limits": [] }"#;
        let snap = parse_body(body).expect("parse");
        assert!(snap.scoped.is_none());
        assert!(snap.weekly.is_none());
    }
}
