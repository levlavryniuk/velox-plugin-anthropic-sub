//! Presenting every request as Claude Code's own.
//!
//! A subscription's tokens are for Claude Code, and Anthropic's API accepts them only from a
//! client that looks like it: its headers, its beta flags, the system block that opens the
//! prompt, the billing header, and the names of its tools. This is that look, layered over the
//! `anthropic` plugin's wire protocol.

use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};
use velox_plugin_anthropic::Access;
use velox_sdk::{
    AuthMethod, Credential, LoginCx, ProviderError, ProviderErrorKind, ToolDeclaration,
};

use crate::billing;
use crate::oauth::OAuth;
use crate::tokens::Tokens;
use crate::tools;
use crate::version::{self, Version};

/// The beta flags a Claude Code request carries: the client itself, and OAuth tokens.
const BETAS: &str = "claude-code-20250219,oauth-2025-04-20";
/// The system block that opens every prompt.
const IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

/// A Claude subscription, reached the way Claude Code reaches it.
pub(crate) struct ClaudeCode {
    oauth: OAuth,
    /// The version the requests claim; raised when Anthropic names a floor, never lowered.
    version: Mutex<Version>,
}

impl ClaudeCode {
    pub(crate) fn new(oauth: OAuth, version: Version) -> Self {
        Self {
            oauth,
            version: Mutex::new(version),
        }
    }

    fn version(&self) -> Version {
        *self.version.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Claims at least `floor` from now on; whether that changed anything.
    fn raise_to(&self, floor: Version) -> bool {
        let mut version = self.version.lock().unwrap_or_else(PoisonError::into_inner);
        let raised = floor > *version;
        if raised {
            *version = floor;
        }
        raised
    }
}

impl Access for ClaudeCode {
    fn methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::OAuth]
    }

    fn headers(
        &self,
        credential: Option<&Credential>,
    ) -> Result<Vec<(String, String)>, ProviderError> {
        let credential = credential
            .ok_or_else(|| ProviderError::new(ProviderErrorKind::Auth, "not logged in"))?;
        let tokens = Tokens::from_credential(credential)?;
        Ok([
            ("authorization", format!("Bearer {}", tokens.access_token)),
            ("anthropic-beta", BETAS.to_owned()),
            ("user-agent", format!("claude-cli/{}", self.version())),
            ("x-app", "cli".to_owned()),
            (
                "anthropic-dangerous-direct-browser-access",
                "true".to_owned(),
            ),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect())
    }

    async fn login(
        &self,
        _provider: &str,
        _method: &AuthMethod,
        cx: &LoginCx,
    ) -> Result<Credential, ProviderError> {
        self.oauth.login(cx).await.map(Tokens::into_credential)
    }

    async fn refresh(&self, credential: Credential) -> Result<Credential, ProviderError> {
        let tokens = Tokens::from_credential(&credential)?;
        self.oauth
            .refresh(&tokens)
            .await
            .map(Tokens::into_credential)
    }

    fn shape_body(&self, body: &mut Value) {
        let billing = billing::header(&first_user_text(body), self.version());
        let own = body
            .get("system")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let system: Vec<Value> = billing
            .into_iter()
            .chain([IDENTITY.to_owned()])
            .chain(own)
            .map(|text| json!({ "type": "text", "text": text }))
            .collect();
        body["system"] = Value::Array(system);
    }

    fn wire_tool_name(&self, name: &str) -> String {
        tools::to_wire(name)
    }

    fn declared_tool_name(&self, wire: &str, declared: &[ToolDeclaration]) -> String {
        tools::from_wire(wire, declared)
    }

    fn rejected(&self, error: ProviderError) -> ProviderError {
        match version::required_by(&error.message) {
            Some(floor) if self.raise_to(floor) => ProviderError {
                retryable: true,
                message: format!(
                    "{} (claiming Claude Code {floor} from now on)",
                    error.message
                ),
                ..error
            },
            _ => error,
        }
    }
}

/// The text of the first block of the first user message, which the billing header hashes.
fn first_user_text(body: &Value) -> String {
    body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|message| message["role"] == "user")
        .and_then(|message| message["content"].as_array())
        .into_iter()
        .flatten()
        .find_map(|block| block["text"].as_str())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(version: Version) -> ClaudeCode {
        let oauth = OAuth::new(reqwest::Client::new(), "http://localhost".to_owned());
        ClaudeCode::new(oauth, version)
    }

    fn too_old(floor: &str) -> ProviderError {
        ProviderError::new(
            ProviderErrorKind::InvalidRequest,
            format!(
                "HTTP 400: Claude Code 2.1.1 does not support this model; version {floor} or newer is required."
            ),
        )
    }

    #[test]
    fn a_floor_above_the_claimed_version_is_claimed_and_the_request_retried() {
        let identity = identity(Version::new(2, 1, 10));

        let error = identity.rejected(too_old("2.1.280"));

        assert!(error.retryable);
        assert_eq!(identity.version(), Version::new(2, 1, 280));
    }

    #[test]
    fn a_floor_the_claimed_version_already_meets_is_not_something_to_retry() {
        let identity = identity(Version::new(2, 1, 300));

        let error = identity.rejected(too_old("2.1.280"));

        assert!(!error.retryable);
        assert_eq!(identity.version(), Version::new(2, 1, 300));
    }

    #[test]
    fn the_first_text_of_the_first_user_message_is_what_gets_hashed() {
        let body = json!({ "messages": [
            { "role": "assistant", "content": [{ "type": "text", "text": "no" }] },
            { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "x", "content": "" },
                { "type": "text", "text": "yes" },
                { "type": "text", "text": "no" },
            ] },
        ] });

        assert_eq!(first_user_text(&body), "yes");
    }
}
