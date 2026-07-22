//! Read the ChatGPT OAuth token written by Codex CLI.
//!
//! Codex stores file-backed credentials in `$CODEX_HOME/auth.json` (normally
//! `~/.codex/auth.json`). API-key logins do not have ChatGPT subscription rate
//! limits, so ccbar intentionally requires the OAuth `tokens` object.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

const ENV_TOKEN: &str = "CCBAR_CODEX_OAUTH_TOKEN";
const ENV_ACCOUNT_ID: &str = "CCBAR_CODEX_ACCOUNT_ID";

#[derive(Debug, Clone)]
pub struct CodexCredentials {
    pub access_token: String,
    pub account_id: Option<String>,
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Environment,
    File,
}

#[derive(Debug, thiserror::Error)]
pub enum CodexCredentialsError {
    #[error("no Codex OAuth credentials found; run `codex login`")]
    NotFound,
    #[error("Codex auth.json read failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Codex auth.json malformed: {0}")]
    Parse(#[from] serde_json::Error),
    #[error(
        "Codex auth.json contains no ChatGPT OAuth token; API-key usage has no subscription quota"
    )]
    MissingOAuthToken,
}

impl CodexCredentials {
    pub fn load() -> Result<Self, CodexCredentialsError> {
        if let Some(credentials) = Self::try_env() {
            return Ok(credentials);
        }

        let path = auth_file_path();
        if !path.exists() {
            return Err(CodexCredentialsError::NotFound);
        }
        parse_payload(&fs::read(path)?, Source::File)
    }

    fn try_env() -> Option<Self> {
        let token = std::env::var(ENV_TOKEN).ok()?;
        let token = token.trim();
        if token.is_empty() {
            return None;
        }
        let account_id = std::env::var(ENV_ACCOUNT_ID)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        Some(Self {
            access_token: token.to_owned(),
            account_id,
            source: Source::Environment,
        })
    }
}

fn auth_file_path() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
        .unwrap_or_else(|| PathBuf::from(".codex"))
        .join("auth.json")
}

fn parse_payload(bytes: &[u8], source: Source) -> Result<CodexCredentials, CodexCredentialsError> {
    #[derive(Deserialize)]
    struct Envelope {
        tokens: Option<Tokens>,
    }

    #[derive(Deserialize)]
    struct Tokens {
        access_token: Option<String>,
        account_id: Option<String>,
    }

    let envelope: Envelope = serde_json::from_slice(bytes)?;
    let tokens = envelope
        .tokens
        .ok_or(CodexCredentialsError::MissingOAuthToken)?;
    let access_token = tokens
        .access_token
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or(CodexCredentialsError::MissingOAuthToken)?;
    let account_id = tokens
        .account_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());

    Ok(CodexCredentials {
        access_token,
        account_id,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_file_backed_oauth_credentials() {
        let body = br#"{
            "OPENAI_API_KEY": null,
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "account_id": "account-123"
            }
        }"#;

        let credentials = parse_payload(body, Source::File).expect("parse");
        assert_eq!(credentials.access_token, "access-token");
        assert_eq!(credentials.account_id.as_deref(), Some("account-123"));
        assert_eq!(credentials.source, Source::File);
    }

    #[test]
    fn rejects_api_key_only_credentials() {
        let body = br#"{ "OPENAI_API_KEY": "test-api-key" }"#;
        assert!(matches!(
            parse_payload(body, Source::File),
            Err(CodexCredentialsError::MissingOAuthToken)
        ));
    }
}
