# velox-plugin-anthropic-sub

A [velox](https://github.com/levlavryniuk/velox) provider plugin that lets you use a **Claude Pro or Max subscription** in velox. It is the `anthropic` plugin's wire protocol (models, thinking, caching, streaming) with a different way in: you sign in with OAuth, and every request presents itself as Claude Code's own.

> **Personal use, at your own risk.** Anthropic's terms say OAuth tokens from Free, Pro and Max plans are for Claude Code and Anthropic's own apps, and that third-party developers may not route requests through those credentials on their users' behalf. This plugin does exactly what the terms describe, as pi's Anthropic login does. It is not part of velox, velox does not ship or endorse it, and Anthropic may refuse or block it, or act against your account, without notice. If you want a supported route, use velox's `anthropic` plugin with an API key. See [the research behind this](https://github.com/levlavryniuk/velox/issues/22).

## Install

```sh
velox plugin add --git https://github.com/levlavryniuk/velox-plugin-anthropic-sub.git
```

or link a checkout, so edits are live after `/reload`:

```sh
velox plugin add --path ~/Coding/velox-plugin-anthropic-sub
```

Then log in, in velox:

```text
/login
```

velox shows a sign-in link. Open it, sign in, and paste the code the page ends with. Models are then `anthropic/claude-…`; pick one with `/model`. The provider has the same id as the `anthropic` plugin's, so turn that plugin off (`enabled = false`) while this one is on. The tokens renew themselves before they expire.

## Config

Every key is optional. In `config.toml`:

```toml
[[plugin]]
id = "anthropic-sub"
source = { git = "https://github.com/levlavryniuk/velox-plugin-anthropic-sub.git" }

[plugin.config]
claude_code_version = "2.1.310"   # the Claude Code version to claim; see below
token_url = "https://platform.claude.com/v1/oauth/token"

[plugin.config.api]               # the `anthropic` plugin's own config
cache_retention = "short"         # short | long | none
base_url = "https://api.anthropic.com"
```

`api` takes the keys of the [`anthropic` plugin's config](https://github.com/levlavryniuk/velox/blob/main/plugins/velox-plugin-anthropic/src/config.rs): `base_url`, `cache_retention` and `prices`. Cost rates are the API's, which is not what a subscription pays; they are for comparing sessions.

## What Anthropic is shown

A subscription's tokens work only for a client that looks like Claude Code. Each request carries:

- headers: `authorization: Bearer <token>`, `user-agent: claude-cli/<version>`, `x-app: cli`, the betas `claude-code-20250219` and `oauth-2025-04-20`;
- a system prompt that opens with the billing header block (`x-anthropic-billing-header: cc_version=…; cc_entrypoint=sdk-cli; cch=…;`) and `You are Claude Code, Anthropic's official CLI for Claude.`, then velox's own prompt;
- tools named the way Claude Code names its own (`read` goes out as `Read`, and the model's `Read` call comes back as `read`).

[`tests/snapshots`](tests/snapshots) holds a request as sent; a change to it is a change to review.

Anthropic holds new models back from old Claude Code versions. When it answers that version `X` or newer is required, the plugin claims `X` from then on and the Core retries the request; `claude_code_version` only sets where it starts. The default is the Claude Code release at the time of writing: raise it if you see `claude_code_version_too_old` failures that do not clear.

## Development

The velox crates are git dependencies on a private repo, so cargo needs access to it (`gh auth setup-git` is enough locally). To build against a local velox checkout, put this in `.cargo/config.toml` (git-ignored):

```toml
[patch."https://github.com/levlavryniuk/velox"]
velox-sdk = { path = "/path/to/velox/crates/velox-sdk" }
velox-plugin-anthropic = { path = "/path/to/velox/plugins/velox-plugin-anthropic" }
velox-core = { path = "/path/to/velox/crates/velox-core" }
velox-test-fakes = { path = "/path/to/velox/crates/velox-test-fakes" }
```

```sh
cargo test                    # unit tests, and the wire and login tests against mock servers
cargo clippy --all-targets
```

CI needs a `VELOX_READ_TOKEN` secret: a token that can read the velox repo.

The login and the wire are tested against mocks. Whether Anthropic accepts a request is only ever known by logging in and trying it.
