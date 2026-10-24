//! A velox provider plugin for Claude through a Claude Pro or Max subscription.
//!
//! It is the `anthropic` plugin's wire protocol with a different way in: the user signs in with
//! OAuth, and every request presents itself as Claude Code's own. That is
//! what pi's Anthropic login does too, and it is against Anthropic's terms for third parties to
//! route requests through subscription credentials. This plugin is for its author's personal
//! use, at their own risk, and is not part of velox.

mod billing;
mod config;
mod identity;
mod oauth;
mod tokens;
mod tools;
mod version;

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use velox_plugin_anthropic::Anthropic;
use velox_sdk::{Plugin, PluginBuilder, PluginError};

pub use crate::config::Config;
use crate::identity::ClaudeCode;
use crate::oauth::OAuth;

/// The plugin id, as used in `[[plugin]] id = "anthropic-sub"`; also the id of its provider, so
/// models are `anthropic-sub/claude-...`.
pub const ID: &str = "anthropic-sub";

fn provider(config: &Config) -> Result<Anthropic<ClaudeCode>, PluginError> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .user_agent(concat!(
            "velox-plugin-anthropic-sub/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|error| PluginError::new(format!("cannot start the HTTP client: {error}")))?;
    let oauth = OAuth::new(client, config.token_url()?);
    let access = ClaudeCode::new(oauth, config.starting_version()?);
    Anthropic::new(ID, config.api(), access)
}

fn builder() -> PluginBuilder<Config> {
    PluginBuilder::<Config>::with_config(ID).providers(|config| Ok(vec![provider(config)?]))
}

/// Builds the plugin.
#[must_use]
pub fn plugin() -> Arc<dyn Plugin> {
    builder().build()
}

/// Runs the plugin as a subprocess of the Core; the body of the `velox-plugin-anthropic-sub`
/// binary.
#[must_use]
pub fn run() -> ExitCode {
    velox_sdk::run(builder())
}
