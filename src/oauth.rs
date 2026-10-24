//! Signing in with a Claude account: OAuth with PKCE against Anthropic's sign-in service, with
//! the public client id Claude Code registered, and the "copy the code" redirect that needs no
//! local server: the page the browser ends up on shows a code for the user to paste back.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use velox_sdk::{LoginCx, ProviderError, ProviderErrorKind};

use crate::tokens::Tokens;

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
pub(crate) const DEFAULT_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const SCOPES: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code \
                      user:mcp_servers user:file_upload";
const INSTRUCTIONS: &str = "Sign in with your Claude account. The page you end up on shows a code: copy it and paste it \
     here.";
/// How many random bytes the PKCE verifier and the state are made of.
const RANDOM_BYTES: usize = 32;

/// The sign-in service.
pub(crate) struct OAuth {
    client: reqwest::Client,
    token_url: String,
}

impl OAuth {
    pub(crate) fn new(client: reqwest::Client, token_url: String) -> Self {
        Self { client, token_url }
    }

    /// Signs the user in: shows the sign-in page, asks for the code it ends with, and trades the
    /// code for tokens.
    pub(crate) async fn login(&self, cx: &LoginCx) -> Result<Tokens, ProviderError> {
        let pkce = Pkce::generate()?;
        let state = random_token()?;

        cx.show_auth_url(
            authorize_url(&pkce.challenge, &state),
            Some(INSTRUCTIONS.to_owned()),
        )
        .await?;
        let pasted = cx.text("Paste the code from the browser").await?;
        let code = pasted_code(&pasted, &state)?;

        cx.show_progress("Signing in").await?;
        self.tokens(&json!({
            "grant_type": "authorization_code",
            "client_id": CLIENT_ID,
            "code": code,
            "state": state,
            "redirect_uri": REDIRECT_URI,
            "code_verifier": pkce.verifier,
        }))
        .await
    }

    /// Renews `current`. The service may rotate the refresh token; when it does the answer must
    /// be stored in its place.
    pub(crate) async fn refresh(&self, current: &Tokens) -> Result<Tokens, ProviderError> {
        self.tokens(&json!({
            "grant_type": "refresh_token",
            "client_id": CLIENT_ID,
            "refresh_token": current.refresh_token,
        }))
        .await
        .map_err(|error| {
            if error.kind == ProviderErrorKind::Auth {
                ProviderError::new(
                    ProviderErrorKind::Auth,
                    format!("{}: log in again with /login", error.message),
                )
            } else {
                error
            }
        })
        .map(|renewed| Tokens {
            refresh_token: if renewed.refresh_token.is_empty() {
                current.refresh_token.clone()
            } else {
                renewed.refresh_token
            },
            ..renewed
        })
    }

    /// `POST` of `body` to the token endpoint.
    async fn tokens(&self, body: &Value) -> Result<Tokens, ProviderError> {
        let response = self
            .client
            .post(&self.token_url)
            .json(body)
            .send()
            .await
            .map_err(|error| ProviderError::new(ProviderErrorKind::Network, error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(token_error(status.as_u16(), &body));
        }
        let answer: TokenAnswer = response.json().await.map_err(|error| {
            ProviderError::new(
                ProviderErrorKind::Other,
                format!("malformed token response: {error}"),
            )
        })?;
        Ok(Tokens {
            access_token: answer.access_token,
            refresh_token: answer.refresh_token.unwrap_or_default(),
            expires_at: unix_now() + answer.expires_in,
        })
    }
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

/// The sign-in page, which asks for what Claude Code asks for.
fn authorize_url(challenge: &str, state: &str) -> String {
    let params = [
        ("code", "true"),
        ("client_id", CLIENT_ID),
        ("response_type", "code"),
        ("redirect_uri", REDIRECT_URI),
        ("scope", SCOPES),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ];
    reqwest::Url::parse_with_params(AUTHORIZE_URL, params)
        .map_or_else(|_| AUTHORIZE_URL.to_owned(), |url| url.to_string())
}

/// The code in what the user pasted: `code#state` as the page shows it, the address it came
/// from, or the bare code.
///
/// # Errors
///
/// Fails with kind `InvalidRequest` when nothing was pasted or the paste belongs to another
/// sign-in.
pub(crate) fn pasted_code(pasted: &str, state: &str) -> Result<String, ProviderError> {
    let invalid = |message: &str| ProviderError::new(ProviderErrorKind::InvalidRequest, message);
    let pasted = pasted.trim();
    if pasted.is_empty() {
        return Err(invalid("nothing was pasted"));
    }
    let (code, found_state) = match reqwest::Url::parse(pasted) {
        Ok(url) => {
            let param = |name: &str| {
                url.query_pairs()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.into_owned())
            };
            (param("code"), param("state"))
        }
        Err(_) => match pasted.split_once('#') {
            Some((code, state)) => (Some(code.to_owned()), Some(state.to_owned())),
            None => (Some(pasted.to_owned()), None),
        },
    };
    match (code, found_state) {
        (_, Some(found)) if found != state => {
            Err(invalid("the pasted code belongs to another sign-in"))
        }
        (Some(code), _) if !code.is_empty() => Ok(code),
        _ => Err(invalid("the pasted address holds no code")),
    }
}

/// The failure of a token request the service answered with `status` and `body`. A refused code
/// or refresh token is the account's to fix by signing in again.
fn token_error(status: u16, body: &str) -> ProviderError {
    #[derive(Deserialize)]
    struct Refusal {
        error: Option<Value>,
        error_description: Option<String>,
    }
    let refusal: Option<Refusal> = serde_json::from_str(body).ok();
    let description = refusal
        .and_then(|refusal| {
            refusal
                .error_description
                .or_else(|| refusal.error.map(|error| error.to_string()))
        })
        .unwrap_or_else(|| body.chars().take(500).collect());
    let kind = match status {
        400 | 401 | 403 => ProviderErrorKind::Auth,
        _ => ProviderErrorKind::Other,
    };
    ProviderError::new(
        kind,
        format!("HTTP {status} from the sign-in service: {description}"),
    )
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// A PKCE pair: the verifier stays here, its hash goes to the sign-in page.
struct Pkce {
    verifier: String,
    challenge: String,
}

impl Pkce {
    fn generate() -> Result<Self, ProviderError> {
        let verifier = random_token()?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Ok(Self {
            verifier,
            challenge,
        })
    }
}

/// An unguessable URL-safe string.
fn random_token() -> Result<String, ProviderError> {
    let mut bytes = [0_u8; RANDOM_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ProviderError::new(
            ProviderErrorKind::Other,
            format!("no source of randomness: {error}"),
        )
    })?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = "state-1";

    fn kind(result: Result<String, ProviderError>) -> ProviderErrorKind {
        result.unwrap_err().kind
    }

    #[test]
    fn the_page_shows_code_and_state_joined_by_a_hash() {
        assert_eq!(pasted_code(" abc#state-1\n", STATE).unwrap(), "abc");
    }

    #[test]
    fn the_address_the_browser_ended_at_holds_the_code_in_its_query() {
        let address = "https://platform.claude.com/oauth/code/callback?code=abc&state=state-1";

        assert_eq!(pasted_code(address, STATE).unwrap(), "abc");
    }

    #[test]
    fn a_bare_code_is_taken_as_it_is() {
        assert_eq!(pasted_code("abc", STATE).unwrap(), "abc");
    }

    #[test]
    fn a_paste_from_another_sign_in_is_refused() {
        assert_eq!(
            kind(pasted_code("abc#someone-else", STATE)),
            ProviderErrorKind::InvalidRequest
        );
        let address = "https://platform.claude.com/oauth/code/callback?code=abc&state=other";
        assert_eq!(
            kind(pasted_code(address, STATE)),
            ProviderErrorKind::InvalidRequest
        );
    }

    #[test]
    fn nothing_or_an_address_without_a_code_is_refused() {
        for pasted in ["", "  ", "https://platform.claude.com/oauth/code/callback"] {
            assert_eq!(
                kind(pasted_code(pasted, STATE)),
                ProviderErrorKind::InvalidRequest,
                "{pasted:?}"
            );
        }
    }

    #[test]
    fn a_refusal_carries_the_services_own_description() {
        let body = r#"{"error":"invalid_grant","error_description":"code expired"}"#;

        let error = token_error(400, body);

        assert_eq!(error.kind, ProviderErrorKind::Auth);
        assert!(error.message.contains("code expired"), "{}", error.message);
    }
}
