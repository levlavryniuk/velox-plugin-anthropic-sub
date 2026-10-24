//! Signing in and renewing, against a mock of Anthropic's sign-in service, through the Core as
//! the Front-end drives it, in-process and as a subprocess.

#![cfg(test)]

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use velox_core::{AuthError, Cancel};
use velox_plugin_anthropic_sub::plugin;
use velox_sdk::{
    AuthMethod, Credential, InitializeParams, LoginNotice, PROTOCOL_VERSION, Plugin,
    ProviderErrorKind,
};
use velox_test_fakes::{Harness, Respond, ScriptedUi, Shown, Transport};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ID: &str = "anthropic-sub";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

/// A sign-in service that answers every token request with `response`.
async fn sign_in_service(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth/token"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

fn tokens() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "access_token": "sk-ant-oat-2",
        "refresh_token": "sk-ant-ort-2",
        "expires_in": 28_800,
    }))
}

/// The JSON of the one token request the service received.
async fn token_request(service: &MockServer) -> Value {
    let received = service.received_requests().await.unwrap();
    assert_eq!(
        received.len(),
        1,
        "expected one token request: {received:?}"
    );
    serde_json::from_slice(&received[0].body).unwrap()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn config(service: &MockServer) -> String {
    format!(
        "[plugin.config]\ntoken_url = \"{}/v1/oauth/token\"\n",
        service.uri()
    )
}

async fn harness(transport: Transport, service: &MockServer) -> Harness {
    let plugin = transport.load(plugin(), env!("CARGO_BIN_EXE_velox-plugin-anthropic-sub"));
    Harness::with_configured_plugins(vec![(ID, &config(service), plugin)]).await
}

async fn log_in(harness: &Harness, ui: &mut ScriptedUi) -> Result<(), AuthError> {
    let cancel = Cancel::new();
    ui.serve(harness.core().login(ID, &AuthMethod::OAuth, &cancel))
        .await
}

/// The query of the page the login sent the user to.
fn sign_in_page(ui: &ScriptedUi) -> std::collections::HashMap<String, String> {
    let url = ui
        .shown()
        .iter()
        .find_map(|shown| match shown {
            Shown::Login {
                notice: LoginNotice::AuthUrl { url, .. },
                ..
            } => Some(url.clone()),
            Shown::Login { .. }
            | Shown::Notify { .. }
            | Shown::Status { .. }
            | Shown::Widget { .. } => None,
        })
        .expect("the login showed no sign-in page");
    let url = reqwest::Url::parse(&url).unwrap();
    assert_eq!(url.host_str(), Some("claude.ai"));
    assert_eq!(url.path(), "/oauth/authorize");
    url.query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

fn failed_with(result: Result<(), AuthError>) -> ProviderErrorKind {
    match result {
        Err(AuthError::Failed(cause)) => cause.kind,
        other => panic!("expected the login to fail, got {other:?}"),
    }
}

fn stored_login() -> Credential {
    Credential {
        expires_at: Some(1),
        data: serde_json::Map::from_iter([
            ("access_token".to_owned(), json!("sk-ant-oat-1")),
            ("refresh_token".to_owned(), json!("sk-ant-ort-1")),
        ]),
    }
}

async fn initialized(transport: Transport, service: &MockServer) -> Arc<dyn Plugin> {
    let plugin = transport.load(plugin(), env!("CARGO_BIN_EXE_velox-plugin-anthropic-sub"));
    plugin
        .initialize(InitializeParams {
            protocol: PROTOCOL_VERSION.to_owned(),
            config: json!({ "token_url": format!("{}/v1/oauth/token", service.uri()) }),
        })
        .await
        .unwrap();
    plugin
}

velox_test_fakes::over_both_transports! {
    async fn a_pasted_code_is_traded_for_tokens_that_are_stored(transport) {
        let service = sign_in_service(tokens()).await;
        let harness = harness(transport, &service).await;
        let mut ui = harness.script_ui([Respond::Input("the-code".to_owned())]);

        log_in(&harness, &mut ui).await.unwrap();

        let stored = harness.stored_credential_of(ID).unwrap();
        assert_eq!(stored.data["access_token"], json!("sk-ant-oat-2"));
        assert_eq!(stored.data["refresh_token"], json!("sk-ant-ort-2"));
        let lasts = stored.expires_at.unwrap().abs_diff(unix_now() + 28_800);
        assert!(lasts < 60, "expires_at is off by {lasts}s");
        let request = token_request(&service).await;
        assert_eq!(request["grant_type"], "authorization_code");
        assert_eq!(request["client_id"], CLIENT_ID);
        assert_eq!(request["code"], "the-code");
    }

    async fn the_sign_in_page_asks_for_what_claude_code_asks_for(transport) {
        let service = sign_in_service(tokens()).await;
        let harness = harness(transport, &service).await;
        let mut ui = harness.script_ui([Respond::Input("the-code".to_owned())]);

        log_in(&harness, &mut ui).await.unwrap();

        let page = sign_in_page(&ui);
        assert_eq!(page["client_id"], CLIENT_ID);
        assert_eq!(page["response_type"], "code");
        assert_eq!(page["code"], "true");
        assert_eq!(page["redirect_uri"], "https://platform.claude.com/oauth/code/callback");
        assert_eq!(page["code_challenge_method"], "S256");
        assert!(page["scope"].split(' ').any(|scope| scope == "user:inference"), "{page:?}");
    }

    async fn the_verifier_sent_with_the_code_is_the_one_the_page_was_challenged_with(transport) {
        let service = sign_in_service(tokens()).await;
        let harness = harness(transport, &service).await;
        let mut ui = harness.script_ui([Respond::Input("the-code".to_owned())]);

        log_in(&harness, &mut ui).await.unwrap();

        let page = sign_in_page(&ui);
        let request = token_request(&service).await;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(request["code_verifier"].as_str().unwrap().as_bytes()));
        assert_eq!(page["code_challenge"], challenge);
        assert_eq!(request["state"], page["state"]);
        assert_eq!(request["redirect_uri"], page["redirect_uri"]);
    }

    async fn a_code_pasted_with_the_state_of_another_sign_in_is_refused(transport) {
        let service = sign_in_service(tokens()).await;
        let harness = harness(transport, &service).await;
        let mut ui = harness.script_ui([Respond::Input("the-code#someone-else".to_owned())]);

        let result = log_in(&harness, &mut ui).await;

        assert_eq!(failed_with(result), ProviderErrorKind::InvalidRequest);
        assert!(service.received_requests().await.unwrap().is_empty());
        assert_eq!(harness.stored_credential_of(ID), None);
    }

    async fn a_user_who_dismisses_the_prompt_ends_the_login(transport) {
        let service = sign_in_service(tokens()).await;
        let harness = harness(transport, &service).await;
        let mut ui = harness.script_ui([Respond::Cancel]);

        let result = log_in(&harness, &mut ui).await;

        assert_eq!(failed_with(result), ProviderErrorKind::Cancelled);
        assert_eq!(harness.stored_credential_of(ID), None);
    }

    async fn a_refused_code_fails_the_login_with_the_services_reason(transport) {
        let refusal = ResponseTemplate::new(400)
            .set_body_json(json!({"error": "invalid_grant", "error_description": "code expired"}));
        let service = sign_in_service(refusal).await;
        let harness = harness(transport, &service).await;
        let mut ui = harness.script_ui([Respond::Input("old-code".to_owned())]);

        let result = log_in(&harness, &mut ui).await;

        let AuthError::Failed(cause) = result.unwrap_err() else { panic!("expected a failed login") };
        assert_eq!(cause.kind, ProviderErrorKind::Auth);
        assert!(cause.message.contains("code expired"), "{}", cause.message);
        assert_eq!(harness.stored_credential_of(ID), None);
    }

    async fn refreshing_trades_the_refresh_token_for_new_tokens(transport) {
        let service = sign_in_service(tokens()).await;
        let plugin = initialized(transport, &service).await;

        let renewed = plugin.refresh(ID, stored_login()).await.unwrap();

        let request = token_request(&service).await;
        assert_eq!(request["grant_type"], "refresh_token");
        assert_eq!(request["refresh_token"], "sk-ant-ort-1");
        assert_eq!(request["client_id"], CLIENT_ID);
        assert_eq!(renewed.data["access_token"], json!("sk-ant-oat-2"));
        assert_eq!(renewed.data["refresh_token"], json!("sk-ant-ort-2"));
        assert!(renewed.expires_at.unwrap() > unix_now());
    }

    async fn a_refresh_that_leaves_out_the_refresh_token_keeps_the_one_it_had(transport) {
        let bare = ResponseTemplate::new(200)
            .set_body_json(json!({"access_token": "sk-ant-oat-3", "expires_in": 60}));
        let service = sign_in_service(bare).await;
        let plugin = initialized(transport, &service).await;

        let renewed = plugin.refresh(ID, stored_login()).await.unwrap();

        assert_eq!(renewed.data["access_token"], json!("sk-ant-oat-3"));
        assert_eq!(renewed.data["refresh_token"], json!("sk-ant-ort-1"));
    }

    async fn a_refresh_the_service_refuses_asks_the_user_to_log_in_again(transport) {
        let refusal = ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"}));
        let service = sign_in_service(refusal).await;
        let plugin = initialized(transport, &service).await;

        let error = plugin.refresh(ID, stored_login()).await.unwrap_err();

        assert_eq!(error.kind, ProviderErrorKind::Auth);
        assert!(error.message.contains("/login"), "{}", error.message);
    }

    async fn an_api_key_is_no_login_to_refresh(transport) {
        let service = sign_in_service(tokens()).await;
        let plugin = initialized(transport, &service).await;

        let error = plugin.refresh(ID, Credential::api_key("sk-ant-api")).await.unwrap_err();

        assert_eq!(error.kind, ProviderErrorKind::Auth);
        assert!(service.received_requests().await.unwrap().is_empty());
    }
}
