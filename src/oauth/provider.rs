//! Provider endpoints and the token exchange. NEROA_OAUTH_LOOPBACK_V11.

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::Request;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

use super::{OAuthConfig, OAuthError, TokenResponse};

/// A provider's OAuth endpoints.
#[derive(Clone, Debug)]
pub struct OAuthProvider {
    pub name: &'static str,
    pub auth_endpoint: &'static str,
    pub token_endpoint: &'static str,
    /// Extra authorization-request params, e.g. Google's access_type=offline
    /// which is what makes it return a refresh token at all.
    pub extra_auth_params: &'static [(&'static str, &'static str)],
}

/// Google, scoped by the caller. Desktop-app client, PKCE, offline access.
pub fn google(client_id: impl Into<String>, scopes: Vec<String>) -> OAuthConfig {
    OAuthConfig {
        provider: OAuthProvider {
            name: "google",
            auth_endpoint: "https://accounts.google.com/o/oauth2/v2/auth",
            token_endpoint: "https://oauth2.googleapis.com/token",
            extra_auth_params: &[("access_type", "offline"), ("prompt", "consent")],
        },
        client_id: client_id.into(),
        client_secret: None,
        scopes,
    }
}

/// GitHub. No verification gate - a registered OAuth app is public
/// immediately. GitHub does not use PKCE, but sending the challenge is
/// harmless and the code path is shared.
pub fn github(client_id: impl Into<String>, scopes: Vec<String>) -> OAuthConfig {
    OAuthConfig {
        provider: OAuthProvider {
            name: "github",
            auth_endpoint: "https://github.com/login/oauth/authorize",
            token_endpoint: "https://github.com/login/oauth/access_token",
            extra_auth_params: &[],
        },
        client_id: client_id.into(),
        client_secret: None,
        scopes,
    }
}

/// Microsoft identity platform (Outlook, OneDrive, Graph). The "common"
/// tenant lets both work and personal accounts sign in. Requires publisher
/// verification before it can go public, like Google.
pub fn microsoft(client_id: impl Into<String>, scopes: Vec<String>) -> OAuthConfig {
    OAuthConfig {
        provider: OAuthProvider {
            name: "microsoft",
            auth_endpoint: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            token_endpoint: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            // offline_access is what yields a refresh token here.
            extra_auth_params: &[("prompt", "select_account")],
        },
        client_id: client_id.into(),
        client_secret: None,
        scopes,
    }
}

/// A provider Neroa does not ship a constructor for. Every OAuth 2.0 +
/// PKCE service fits this: supply its two endpoints and you are done, which
/// is the whole point of the loopback design - the mechanism is universal,
/// only the endpoints and scopes differ.
pub fn custom(
    name: &'static str,
    auth_endpoint: &'static str,
    token_endpoint: &'static str,
    client_id: impl Into<String>,
    scopes: Vec<String>,
) -> OAuthConfig {
    OAuthConfig {
        provider: OAuthProvider {
            name,
            auth_endpoint,
            token_endpoint,
            extra_auth_params: &[],
        },
        client_id: client_id.into(),
        client_secret: None,
        scopes,
    }
}

/// Build the consent URL the user opens in their real browser.
pub fn authorization_url(
    config: &OAuthConfig,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> String {
    let scope = config.scopes.join(" ");

    let mut params = vec![
        ("client_id", config.client_id.as_str()),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("scope", scope.as_str()),
    ];

    for (key, value) in config.provider.extra_auth_params {
        params.push((key, value));
    }

    let query = params
        .iter()
        .map(|(key, value)| format!("{key}={}", urlencode(value)))
        .collect::<Vec<_>>()
        .join("&");

    format!("{}?{}", config.provider.auth_endpoint, query)
}

pub async fn exchange_code(
    config: &OAuthConfig,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<TokenResponse, OAuthError> {
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("client_id", config.client_id.clone()),
        ("code_verifier", verifier.to_string()),
    ];

    if let Some(secret) = &config.client_secret {
        form.push(("client_secret", secret.clone()));
    }

    post_form(config.provider.token_endpoint, &form).await
}

pub async fn refresh_token(
    config: &OAuthConfig,
    refresh_token: &str,
) -> Result<TokenResponse, OAuthError> {
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
        ("client_id", config.client_id.clone()),
    ];

    if let Some(secret) = &config.client_secret {
        form.push(("client_secret", secret.clone()));
    }

    post_form(config.provider.token_endpoint, &form).await
}

async fn post_form(endpoint: &str, form: &[(&str, String)]) -> Result<TokenResponse, OAuthError> {
    let body = form
        .iter()
        .map(|(key, value)| format!("{key}={}", urlencode(value)))
        .collect::<Vec<_>>()
        .join("&");

    let connector = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_only()
        .enable_http1()
        .build();

    let client: Client<_, Full<Bytes>> = Client::builder(TokioExecutor::new()).build(connector);

    let request = Request::builder()
        .method("POST")
        .uri(endpoint)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body(Full::new(Bytes::from(body)))
        .map_err(|error| OAuthError::Http(format!("request build: {error}")))?;

    let response = client
        .request(request)
        .await
        .map_err(|error| OAuthError::Http(format!("token request: {error}")))?;

    let status = response.status();

    let bytes = response
        .into_body()
        .collect()
        .await
        .map_err(|error| OAuthError::Http(format!("read body: {error}")))?
        .to_bytes();

    let text = String::from_utf8_lossy(&bytes);

    if !status.is_success() {
        // Google returns {"error":"...","error_description":"..."} with a 4xx.
        if let Ok(err) = serde_json::from_str::<ProviderError>(&text) {
            return Err(OAuthError::Provider {
                error: err.error,
                description: err.error_description,
            });
        }

        return Err(OAuthError::Http(format!("token endpoint {status}: {text}")));
    }

    serde_json::from_str::<TokenResponse>(&text)
        .map_err(|error| OAuthError::Http(format!("decode token response: {error}: {text}")))
}

#[derive(serde::Deserialize)]
struct ProviderError {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// application/x-www-form-urlencoded value encoding.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());

    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }

    out
}
