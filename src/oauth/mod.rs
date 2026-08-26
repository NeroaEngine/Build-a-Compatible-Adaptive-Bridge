//! Loopback OAuth 2.0 for native apps (RFC 8252) with PKCE (RFC 7636).
//!
//! NEROA_OAUTH_LOOPBACK_V11
//!
//! The provider's own login page will not run inside Servo - Cloudflare and
//! Google-grade bot checks fingerprint the engine and reject it. So consent
//! happens in the user's real browser, where those checks pass, and the token
//! comes back to a loopback server Neroa runs for the length of one sign-in.
//! This is exactly how gcloud, gh, and the cloud CLIs authenticate.
//!
//! What this grants is a token for the provider's API (Gmail, Drive, the
//! OpenAI API), not a logged-in session inside Neroa's web view. The two are
//! different things: the API token is what feeds data into the graph and acts
//! on the user's behalf; it does not make Servo a signed-in consumer browser.
//!
//! Security properties this holds to:
//!  * PKCE S256, so an intercepted authorization code is useless without the
//!    verifier, which never leaves this process.
//!  * A `state` nonce, checked on the redirect, so a stray request cannot
//!    inject a code.
//!  * The loopback server binds 127.0.0.1 only and answers exactly one
//!    request before shutting down.
//!  * The client secret, where a provider requires one for a "desktop app",
//!    is treated as non-confidential per RFC 8252 section 8.5 - it is not a
//!    real secret in a native app and nothing here pretends otherwise.

mod pkce;
mod provider;
mod server;
mod token;

pub use provider::{OAuthProvider, custom, github, google, microsoft};
pub use token::{TokenSet, TokenStore};

use std::time::Duration;

use serde::Deserialize;

/// Everything a sign-in needs.
#[derive(Clone, Debug)]
pub struct OAuthConfig {
    pub provider: OAuthProvider,

    pub client_id: String,

    /// Present only for providers that issue a "desktop app" secret. Not
    /// confidential in a native app; see the module note.
    pub client_secret: Option<String>,

    pub scopes: Vec<String>,
}

#[derive(Debug)]
pub enum OAuthError {
    Io(std::io::Error),
    Http(String),
    Provider { error: String, description: Option<String> },
    StateMismatch,
    Timeout,
    Cancelled,
}

impl std::fmt::Display for OAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OAuthError::Io(error) => write!(f, "io: {error}"),
            OAuthError::Http(message) => write!(f, "http: {message}"),
            OAuthError::Provider { error, description } => {
                write!(f, "provider: {error}")?;
                if let Some(description) = description {
                    write!(f, " ({description})")?;
                }
                Ok(())
            }
            OAuthError::StateMismatch => write!(f, "redirect state did not match request"),
            OAuthError::Timeout => write!(f, "timed out waiting for the browser redirect"),
            OAuthError::Cancelled => write!(f, "sign-in was cancelled"),
        }
    }
}

impl std::error::Error for OAuthError {}

impl From<std::io::Error> for OAuthError {
    fn from(error: std::io::Error) -> Self {
        OAuthError::Io(error)
    }
}

/// The URL the user must open to consent, plus the state the caller needs to
/// complete the exchange. Returned by [`begin`] so the caller decides how to
/// open the browser (Neroa uses ShellExecute to the real Chrome).
pub struct PendingAuth {
    /// Open this in the user's real browser.
    pub authorization_url: String,

    /// The loopback port the server is listening on.
    pub redirect_port: u16,

    config: OAuthConfig,
    verifier: String,
    state: String,
    redirect_uri: String,
}

/// Raw token response from the provider's token endpoint.
#[derive(Debug, Deserialize)]
pub(crate) struct TokenResponse {
    pub access_token: String,

    #[serde(default)]
    pub refresh_token: Option<String>,

    #[serde(default)]
    pub expires_in: Option<u64>,

    #[serde(default)]
    pub token_type: Option<String>,

    #[serde(default)]
    pub scope: Option<String>,
}

/// Start a sign-in: bind the loopback port and build the consent URL.
///
/// Does not open a browser or block. The caller opens `authorization_url`,
/// then calls [`complete`] to wait for the redirect and exchange the code.
pub fn begin(config: OAuthConfig) -> Result<(PendingAuth, server::LoopbackServer), OAuthError> {
    let server = server::LoopbackServer::bind()?;

    let port = server.port();

    let redirect_uri = format!("http://127.0.0.1:{port}");

    let verifier = pkce::verifier();

    let challenge = pkce::challenge(&verifier);

    let state = pkce::nonce();

    let authorization_url = provider::authorization_url(
        &config,
        &redirect_uri,
        &challenge,
        &state,
    );

    Ok((
        PendingAuth {
            authorization_url,
            redirect_port: port,
            config,
            verifier,
            state,
            redirect_uri,
        },
        server,
    ))
}

/// Wait for the browser redirect and exchange the code for tokens.
///
/// Blocks up to `timeout` for the user to finish signing in.
pub async fn complete(
    pending: PendingAuth,
    server: server::LoopbackServer,
    timeout: Duration,
) -> Result<TokenSet, OAuthError> {
    let redirect = server.wait_for_redirect(timeout).await?;

    if redirect.state != pending.state {
        return Err(OAuthError::StateMismatch);
    }

    if let Some(error) = redirect.error {
        return Err(OAuthError::Provider {
            error,
            description: redirect.error_description,
        });
    }

    let code = redirect.code.ok_or_else(|| OAuthError::Provider {
        error: "no_code".to_string(),
        description: Some("redirect carried neither a code nor an error".to_string()),
    })?;

    let response = provider::exchange_code(
        &pending.config,
        &pending.redirect_uri,
        &code,
        &pending.verifier,
    )
    .await?;

    Ok(TokenSet::from_response(response, &pending.config.scopes))
}

/// Refresh an expired access token without user interaction.
pub async fn refresh(config: &OAuthConfig, refresh_token: &str) -> Result<TokenSet, OAuthError> {
    let response = provider::refresh_token(config, refresh_token).await?;

    let mut tokens = TokenSet::from_response(response, &config.scopes);

    // Providers commonly omit the refresh token on refresh; keep the old one.
    if tokens.refresh_token.is_none() {
        tokens.refresh_token = Some(refresh_token.to_string());
    }

    Ok(tokens)
}
