//! Token set and on-disk store. NEROA_OAUTH_LOOPBACK_V11.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{OAuthError, TokenResponse};

/// The result of a successful sign-in.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,

    #[serde(default)]
    pub refresh_token: Option<String>,

    /// Absolute expiry as unix seconds, computed from expires_in at receipt.
    /// Absolute rather than relative so a stored token can be checked later
    /// without knowing when it was issued.
    #[serde(default)]
    pub expires_at: Option<u64>,

    #[serde(default)]
    pub token_type: Option<String>,

    #[serde(default)]
    pub scopes: Vec<String>,
}

impl TokenSet {
    pub(crate) fn from_response(response: TokenResponse, requested: &[String]) -> Self {
        let expires_at = response.expires_in.map(|seconds| now() + seconds);

        let scopes = response
            .scope
            .map(|scope| scope.split_whitespace().map(str::to_string).collect())
            .unwrap_or_else(|| requested.to_vec());

        Self {
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            expires_at,
            token_type: response.token_type,
            scopes,
        }
    }

    /// True if the access token has expired or is within `skew` of doing so.
    /// The skew keeps a token from being used in the seconds before it dies.
    pub fn is_expired(&self, skew_seconds: u64) -> bool {
        match self.expires_at {
            Some(at) => now() + skew_seconds >= at,
            None => false,
        }
    }
}

/// Persists token sets under the Neroa profile, one JSON file per provider.
pub struct TokenStore {
    dir: PathBuf,
}

impl TokenStore {
    /// Store rooted at the given directory (the Neroa profile in practice).
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().join("oauth"),
        }
    }

    pub fn save(&self, provider: &str, tokens: &TokenSet) -> Result<(), OAuthError> {
        std::fs::create_dir_all(&self.dir)?;

        let path = self.path(provider);

        let json = serde_json::to_string_pretty(tokens)
            .map_err(|error| OAuthError::Http(format!("encode tokens: {error}")))?;

        // Write-then-rename so a crash mid-write cannot corrupt a good token.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;

        Ok(())
    }

    pub fn load(&self, provider: &str) -> Option<TokenSet> {
        let text = std::fs::read_to_string(self.path(provider)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn delete(&self, provider: &str) -> Result<(), OAuthError> {
        match std::fs::remove_file(self.path(provider)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(OAuthError::Io(error)),
        }
    }

    fn path(&self, provider: &str) -> PathBuf {
        let safe: String = provider
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();

        self.dir.join(format!("{safe}.json"))
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
