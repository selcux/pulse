use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Token types (field names match garth for compatibility)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConsumer {
    pub consumer_key: String,
    pub consumer_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth1Token {
    pub oauth_token: String,
    pub oauth_token_secret: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mfa_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mfa_expiration_timestamp: Option<String>,
    #[serde(default = "default_domain")]
    pub domain: String,
}

fn default_domain() -> String {
    "garmin.com".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Token {
    pub scope: String,
    pub jti: String,
    pub token_type: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    #[serde(default)]
    pub expires_at: i64,
    pub refresh_token_expires_in: i64,
    #[serde(default)]
    pub refresh_token_expires_at: i64,
}

impl OAuth2Token {
    /// Compute expires_at from expires_in if not set
    pub fn compute_expirations(&mut self) {
        let now = chrono::Utc::now().timestamp();
        if self.expires_at == 0 {
            self.expires_at = now + self.expires_in;
        }
        if self.refresh_token_expires_at == 0 {
            self.refresh_token_expires_at = now + self.refresh_token_expires_in;
        }
    }
}

impl OAuth2Token {
    /// Returns true if the access token is expired (with 60s buffer).
    pub fn is_expired(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        self.expires_at < now + 60
    }

    /// Returns true if the refresh token is also expired.
    pub fn is_refresh_expired(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        self.refresh_token_expires_at < now + 60
    }
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

/// Returns the token storage directory: ~/.pulse/garmin/
pub fn token_dir() -> PathBuf {
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".pulse")
        .join("garmin")
}

/// Save OAuth1 token to disk.
pub fn save_oauth1(token: &OAuth1Token) -> Result<()> {
    let dir = token_dir();
    std::fs::create_dir_all(&dir).context("Failed to create garmin token directory")?;
    let path = dir.join("oauth1_token.json");
    let json = serde_json::to_string_pretty(token)?;
    std::fs::write(&path, json).context("Failed to write oauth1_token.json")?;
    Ok(())
}

/// Load OAuth1 token from disk.
pub fn load_oauth1() -> Result<OAuth1Token> {
    let path = token_dir().join("oauth1_token.json");
    let json = std::fs::read_to_string(&path).context("No OAuth1 token found. Run `pulse garmin-login` first.")?;
    let token: OAuth1Token = serde_json::from_str(&json).context("Failed to parse oauth1_token.json")?;
    Ok(token)
}

/// Save OAuth2 token to disk.
pub fn save_oauth2(token: &OAuth2Token) -> Result<()> {
    let dir = token_dir();
    std::fs::create_dir_all(&dir).context("Failed to create garmin token directory")?;
    let path = dir.join("oauth2_token.json");
    let json = serde_json::to_string_pretty(token)?;
    std::fs::write(&path, json).context("Failed to write oauth2_token.json")?;
    Ok(())
}

/// Load OAuth2 token from disk.
pub fn load_oauth2() -> Result<OAuth2Token> {
    let path = token_dir().join("oauth2_token.json");
    let json = std::fs::read_to_string(&path).context("No OAuth2 token found. Run `pulse garmin-login` first.")?;
    let token: OAuth2Token = serde_json::from_str(&json).context("Failed to parse oauth2_token.json")?;
    Ok(token)
}

/// Quick check: do both token files exist?
pub fn tokens_exist() -> bool {
    let dir = token_dir();
    dir.join("oauth1_token.json").exists() && dir.join("oauth2_token.json").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth2_token_expiry_check() {
        let now = chrono::Utc::now().timestamp();
        let token = OAuth2Token {
            scope: "test".into(),
            jti: "jti".into(),
            token_type: "Bearer".into(),
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            expires_in: 3600,
            expires_at: now + 3600,
            refresh_token_expires_in: 7776000,
            refresh_token_expires_at: now + 7776000,
        };
        assert!(!token.is_expired());
        assert!(!token.is_refresh_expired());
    }

    #[test]
    fn oauth2_token_detects_expired() {
        let now = chrono::Utc::now().timestamp();
        let token = OAuth2Token {
            scope: "test".into(),
            jti: "jti".into(),
            token_type: "Bearer".into(),
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            expires_in: 3600,
            expires_at: now - 10, // already expired
            refresh_token_expires_in: 7776000,
            refresh_token_expires_at: now + 7776000,
        };
        assert!(token.is_expired());
        assert!(!token.is_refresh_expired());
    }

    #[test]
    fn oauth2_token_expiry_buffer() {
        let now = chrono::Utc::now().timestamp();
        // 30 seconds left — within 60s buffer, should report expired
        let token = OAuth2Token {
            scope: "test".into(),
            jti: "jti".into(),
            token_type: "Bearer".into(),
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            expires_in: 3600,
            expires_at: now + 30,
            refresh_token_expires_in: 7776000,
            refresh_token_expires_at: now + 7776000,
        };
        assert!(token.is_expired());
    }

    #[test]
    fn oauth1_token_roundtrip_json() {
        let token = OAuth1Token {
            oauth_token: "tok123".into(),
            oauth_token_secret: "sec456".into(),
            mfa_token: None,
            mfa_expiration_timestamp: None,
            domain: "garmin.com".into(),
        };
        let json = serde_json::to_string(&token).unwrap();
        let parsed: OAuth1Token = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.oauth_token, "tok123");
        assert_eq!(parsed.oauth_token_secret, "sec456");
        assert_eq!(parsed.domain, "garmin.com");
    }

    #[test]
    fn oauth2_token_roundtrip_json() {
        let now = chrono::Utc::now().timestamp();
        let token = OAuth2Token {
            scope: "CONNECT_READ CONNECT_WRITE".into(),
            jti: "abc-123".into(),
            token_type: "Bearer".into(),
            access_token: "eyJ...".into(),
            refresh_token: "ref...".into(),
            expires_in: 3600,
            expires_at: now + 3600,
            refresh_token_expires_in: 7776000,
            refresh_token_expires_at: now + 7776000,
        };
        let json = serde_json::to_string_pretty(&token).unwrap();
        let parsed: OAuth2Token = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.token_type, "Bearer");
        assert_eq!(parsed.access_token, "eyJ...");
    }

    #[test]
    fn oauth_consumer_deserialize() {
        let json = r#"{"consumer_key":"fc3e99d2","consumer_secret":"E08WAR897"}"#;
        let consumer: OAuthConsumer = serde_json::from_str(json).unwrap();
        assert_eq!(consumer.consumer_key, "fc3e99d2");
        assert_eq!(consumer.consumer_secret, "E08WAR897");
    }

    #[test]
    fn token_dir_is_under_pulse() {
        let dir = token_dir();
        assert!(dir.to_string_lossy().contains(".pulse"));
        assert!(dir.to_string_lossy().ends_with("garmin"));
    }
}
