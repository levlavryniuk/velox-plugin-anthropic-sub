//! The credential this provider stores: the tokens of a signed-in Claude account.

use serde_json::Value;
use velox_sdk::{Credential, ProviderError, ProviderErrorKind};

const ACCESS_TOKEN: &str = "access_token";
const REFRESH_TOKEN: &str = "refresh_token";

/// What a login or a refresh gave: tokens, and when the access token stops working.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tokens {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    /// In Unix seconds.
    pub(crate) expires_at: u64,
}

impl Tokens {
    pub(crate) fn into_credential(self) -> Credential {
        Credential {
            expires_at: Some(self.expires_at),
            data: [
                (ACCESS_TOKEN, self.access_token),
                (REFRESH_TOKEN, self.refresh_token),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), Value::String(value)))
            .collect(),
        }
    }

    /// The login `credential` stores.
    ///
    /// # Errors
    ///
    /// Fails with kind `Auth` when `credential` is not one, such as an API key.
    pub(crate) fn from_credential(credential: &Credential) -> Result<Self, ProviderError> {
        Self::read(credential).ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::Auth,
                "the stored credential is not a Claude login: log in again",
            )
        })
    }

    fn read(credential: &Credential) -> Option<Self> {
        let field = |key: &str| credential.data.get(key)?.as_str().map(str::to_owned);
        Some(Self {
            access_token: field(ACCESS_TOKEN)?,
            refresh_token: field(REFRESH_TOKEN)?,
            expires_at: credential.expires_at?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> Tokens {
        Tokens {
            access_token: "sk-ant-oat-1".to_owned(),
            refresh_token: "sk-ant-ort-1".to_owned(),
            expires_at: 1_700_000_000,
        }
    }

    #[test]
    fn a_stored_login_reads_back_as_it_was_made() {
        let credential = tokens().into_credential();

        assert_eq!(Tokens::from_credential(&credential).unwrap(), tokens());
    }

    #[test]
    fn an_api_key_is_not_a_login() {
        let error = Tokens::from_credential(&Credential::api_key("sk-ant-api")).unwrap_err();

        assert_eq!(error.kind, ProviderErrorKind::Auth);
    }
}
