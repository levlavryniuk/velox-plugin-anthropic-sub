//! The plugin's `[plugin.config]` section.

use std::str::FromStr;

use schemars::JsonSchema;
use serde::Deserialize;
use velox_plugin_anthropic::Config as ApiConfig;
use velox_sdk::PluginError;

use crate::oauth::DEFAULT_TOKEN_URL;
use crate::version::{self, Version};

/// The plugin's config; every key is optional.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The Claude Code version the plugin says it is, as `major.minor.patch`. Anthropic refuses
    /// some models to versions that are too old; when it does, the plugin raises the version on
    /// its own, so this is only for starting higher.
    claude_code_version: Option<String>,
    /// Where the sign-in service exchanges and renews tokens. Defaults to Anthropic's.
    token_url: Option<String>,
    /// How the Messages API is reached and what it costs, with the keys of the `anthropic`
    /// plugin's own config (`base_url`, `cache_retention`, `prices`).
    #[serde(default)]
    api: ApiConfig,
}

impl Config {
    pub(crate) fn api(&self) -> &ApiConfig {
        &self.api
    }

    /// The version to start from.
    ///
    /// # Errors
    ///
    /// Fails when `claude_code_version` is not a bare `major.minor.patch`.
    pub(crate) fn starting_version(&self) -> Result<Version, PluginError> {
        self.claude_code_version
            .as_deref()
            .map_or(Ok(version::DEFAULT), |text| {
                Version::from_str(text).map_err(|()| {
                    PluginError::new(format!(
                        "invalid claude_code_version `{text}`: use major.minor.patch, like 2.1.310"
                    ))
                })
            })
    }

    /// The token endpoint's URL.
    ///
    /// # Errors
    ///
    /// Fails when `token_url` is not a URL.
    pub(crate) fn token_url(&self) -> Result<String, PluginError> {
        let url = self.token_url.as_deref().unwrap_or(DEFAULT_TOKEN_URL);
        reqwest::Url::parse(url)
            .map(|_| url.to_owned())
            .map_err(|_| PluginError::new(format!("invalid token_url `{url}`")))
    }
}
