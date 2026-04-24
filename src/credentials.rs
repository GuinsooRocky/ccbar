//! Read Claude OAuth credentials from the macOS Keychain (primary) or the
//! `~/.claude/.credentials.json` file fallback.
//!
//! Shape produced by `claude login` / `claude setup-token`:
//!
//! ```json
//! { "claudeAiOauth": {
//!     "accessToken":  "sk-ant-oat01-...",
//!     "refreshToken": "sk-ant-ort01-...",
//!     "expiresAt":    1735689600000,
//!     "scopes":       ["user:inference", "user:profile"],
//!     "rateLimitTier": "claude_max_20x"
//! }}
//! ```
//!
//! Keychain service name: `Claude Code-credentials` (see CodexBar
//! `ClaudeOAuthCredentials.swift:16`).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;

const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
const FILE_FALLBACK: &str = ".claude/.credentials.json";
const ENV_TOKEN: &str = "CCBAR_CLAUDE_OAUTH_TOKEN";
const ENV_SCOPES: &str = "CCBAR_CLAUDE_OAUTH_SCOPES";

#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix epoch milliseconds. `None` means the credential never declared a
    /// TTL (some environment-var overrides).
    pub expires_at_ms: Option<i64>,
    pub scopes: Vec<String>,
    pub rate_limit_tier: Option<String>,
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Environment,
    Keychain,
    File,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialsError {
    #[error("no Claude OAuth credentials found in env, Keychain, or ~/.claude/.credentials.json")]
    NotFound,
    #[error("/usr/bin/security failed (exit {code:?}): {stderr}")]
    SecurityCli { code: Option<i32>, stderr: String },
    #[error("credential JSON malformed: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("credential file read failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("credential JSON missing required field: {0}")]
    MissingField(&'static str),
    #[error("credential access token missing or empty")]
    MissingAccessToken,
}

impl Credentials {
    pub fn load() -> Result<Self, CredentialsError> {
        if let Some(c) = Self::try_env() {
            return Ok(c);
        }
        match Self::try_keychain() {
            Ok(Some(c)) => return Ok(c),
            Ok(None) => {}
            Err(e) => {
                // Keychain errors are non-fatal: fall through to file.
                eprintln!("ccbar: keychain read failed: {e}");
            }
        }
        if let Some(c) = Self::try_file()? {
            return Ok(c);
        }
        Err(CredentialsError::NotFound)
    }

    pub fn is_expired(&self, now_ms: i64) -> bool {
        self.expires_at_ms.is_some_and(|t| now_ms >= t)
    }

    pub fn has_user_profile_scope(&self) -> bool {
        self.scopes.iter().any(|s| s == "user:profile")
    }

    fn try_env() -> Option<Self> {
        let token = std::env::var(ENV_TOKEN).ok()?;
        let token = token.trim();
        if token.is_empty() {
            return None;
        }
        let scopes = std::env::var(ENV_SCOPES)
            .ok()
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Some(Self {
            access_token: token.to_owned(),
            refresh_token: None,
            expires_at_ms: None,
            scopes,
            rate_limit_tier: None,
            source: Source::Environment,
        })
    }

    fn try_keychain() -> Result<Option<Self>, CredentialsError> {
        // Shelling out to /usr/bin/security avoids the double ACL prompt we hit
        // when SecItemCopyMatching is called with kSecReturnData=true, and lets
        // macOS pin the trust to a system binary whose signature never changes.
        let output = Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
            .output()?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let trimmed = stdout.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let creds = parse_payload(trimmed.as_bytes(), Source::Keychain)?;
            return Ok(Some(creds));
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        // Exit 44 / "could not be found" → no such Keychain item, treat as None.
        if stderr.contains("could not be found") {
            return Ok(None);
        }
        Err(CredentialsError::SecurityCli {
            code: output.status.code(),
            stderr,
        })
    }

    fn try_file() -> Result<Option<Self>, CredentialsError> {
        let path = credentials_file_path();
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path)?;
        let creds = parse_payload(&bytes, Source::File)?;
        Ok(Some(creds))
    }
}

fn credentials_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(FILE_FALLBACK)
}

fn parse_payload(bytes: &[u8], source: Source) -> Result<Credentials, CredentialsError> {
    #[derive(Deserialize)]
    struct Envelope {
        #[serde(rename = "claudeAiOauth")]
        claude_ai_oauth: Option<Oauth>,
    }
    #[derive(Deserialize)]
    struct Oauth {
        #[serde(rename = "accessToken")]
        access_token: Option<String>,
        #[serde(rename = "refreshToken")]
        refresh_token: Option<String>,
        #[serde(rename = "expiresAt")]
        expires_at: Option<i64>,
        #[serde(default)]
        scopes: Vec<String>,
        #[serde(rename = "rateLimitTier")]
        rate_limit_tier: Option<String>,
    }

    let env: Envelope = serde_json::from_slice(bytes)?;
    let oauth = env
        .claude_ai_oauth
        .ok_or(CredentialsError::MissingField("claudeAiOauth"))?;
    let access_token = oauth
        .access_token
        .filter(|s| !s.trim().is_empty())
        .ok_or(CredentialsError::MissingAccessToken)?;

    Ok(Credentials {
        access_token,
        refresh_token: oauth.refresh_token.filter(|s| !s.trim().is_empty()),
        expires_at_ms: oauth.expires_at,
        scopes: oauth.scopes,
        rate_limit_tier: oauth.rate_limit_tier,
        source,
    })
}
