//! What a request looks like on the wire: the plugin through the wire `Plugin` trait, against a
//! local mock of the Messages API, in-process and as a subprocess. The Claude Code identity is
//! snapshotted, so a change to what Anthropic is shown is a change to review.

#![cfg(test)]

use std::sync::Arc;

use serde_json::{Value, json};
use velox_plugin_anthropic_sub::plugin;
use velox_sdk::{
    Credential, EventSink, ExecutionMode, InitializeParams, Message, PROTOCOL_VERSION, Plugin,
    ProviderCx, ProviderError, ProviderErrorKind, ProviderEvent, Request, ThinkingLevel,
    ToolDeclaration,
};
use velox_test_fakes::Transport;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PROVIDER: &str = "anthropic";
const MODELS: &str = include_str!("fixtures/models.json");
const TEXT: &str = include_str!("fixtures/text.sse");
const TOOL_CALL: &str = include_str!("fixtures/tool_call.sse");

/// The request headers that say who is asking.
const IDENTITY_HEADERS: [&str; 6] = [
    "anthropic-beta",
    "anthropic-dangerous-direct-browser-access",
    "anthropic-version",
    "authorization",
    "user-agent",
    "x-app",
];

struct Endpoint {
    server: MockServer,
    plugin: Arc<dyn Plugin>,
}

fn sse(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "text/event-stream")
}

/// An API that lists its models to a logged-in client only, and answers messages with `reply`.
async fn endpoint(transport: Transport, reply: ResponseTemplate) -> Endpoint {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer sk-ant-oat-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(MODELS.as_bytes().to_vec(), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(reply)
        .mount(&server)
        .await;
    Endpoint {
        plugin: initialized(transport, &server).await,
        server,
    }
}

async fn initialized(transport: Transport, server: &MockServer) -> Arc<dyn Plugin> {
    let plugin = transport.load(plugin(), env!("CARGO_BIN_EXE_velox-plugin-anthropic-sub"));
    plugin
        .initialize(InitializeParams {
            protocol: PROTOCOL_VERSION.to_owned(),
            config: json!({ "api": { "base_url": server.uri() } }),
        })
        .await
        .unwrap();
    plugin
}

fn login() -> Credential {
    Credential {
        expires_at: Some(4_000_000_000),
        data: serde_json::Map::from_iter([
            ("access_token".to_owned(), json!("sk-ant-oat-1")),
            ("refresh_token".to_owned(), json!("sk-ant-ort-1")),
        ]),
    }
}

fn cx() -> ProviderCx {
    ProviderCx::default().with_credential(Some(login()))
}

fn tool(name: &str) -> ToolDeclaration {
    ToolDeclaration {
        name: name.to_owned(),
        description: format!("The {name} tool."),
        parameters: json!({ "type": "object" }),
        mode: ExecutionMode::Parallel,
    }
}

fn request(tools: Vec<ToolDeclaration>) -> Request {
    Request {
        model: "claude-sonnet-4-6".to_owned(),
        system: "Be brief.".to_owned(),
        messages: vec![Message::user("Hello, world! this is the first message")],
        tools,
        thinking: ThinkingLevel::Off,
        max_output: Some(1024),
    }
}

async fn stream(
    endpoint: &Endpoint,
    request: Request,
) -> (Vec<ProviderEvent>, Result<(), ProviderError>) {
    let (sink, mut events) = EventSink::channel();
    let result = endpoint.plugin.stream(PROVIDER, request, &cx(), sink).await;
    let mut collected = Vec::new();
    while let Some(event) = events.try_recv() {
        collected.push(event);
    }
    (collected, result)
}

/// The message requests the endpoint received, as their identity headers and body.
async fn sent(endpoint: &Endpoint) -> Vec<Value> {
    endpoint
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.method.as_str() == "POST")
        .map(|request| {
            let headers: serde_json::Map<String, Value> = IDENTITY_HEADERS
                .iter()
                .filter_map(|name| {
                    let value = request.headers.get(*name)?.to_str().ok()?;
                    Some(((*name).to_owned(), json!(value)))
                })
                .collect();
            json!({
                "headers": headers,
                "body": serde_json::from_slice::<Value>(&request.body).unwrap(),
            })
        })
        .collect()
}

fn called(events: &[ProviderEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match event {
            ProviderEvent::ToolCall(call) => Some(call.name.as_str()),
            ProviderEvent::TextDelta { .. }
            | ProviderEvent::ThinkingDelta { .. }
            | ProviderEvent::ThinkingEnd { .. }
            | ProviderEvent::Usage(_)
            | ProviderEvent::Done { .. } => None,
        })
        .collect()
}

velox_test_fakes::over_both_transports! {
    async fn a_request_presents_itself_as_claude_code(transport) {
        let endpoint = endpoint(transport, sse(TEXT)).await;

        let (_, result) = stream(&endpoint, request(vec![tool("read"), tool("lint")])).await;

        result.unwrap();
        insta::assert_json_snapshot!("claude_code_request", sent(&endpoint).await);
    }

    async fn models_are_listed_with_the_login(transport) {
        let endpoint = endpoint(transport, sse(TEXT)).await;

        let models = endpoint.plugin.models(PROVIDER, &cx()).await.unwrap();

        assert!(models.iter().any(|model| model.id == "claude-sonnet-4-6"));
    }

    async fn tools_go_out_in_claude_codes_casing_and_calls_come_back_in_velox_s(transport) {
        let endpoint = endpoint(transport, sse(TOOL_CALL)).await;

        let (events, result) = stream(&endpoint, request(vec![tool("read")])).await;

        result.unwrap();
        assert_eq!(sent(&endpoint).await[0]["body"]["tools"][0]["name"], "Read");
        assert_eq!(called(&events), ["read"]);
    }

    async fn a_request_with_no_login_stored_is_refused_before_it_is_sent(transport) {
        let endpoint = endpoint(transport, sse(TEXT)).await;
        let (sink, _events) = EventSink::channel();

        let error = endpoint
            .plugin
            .stream(PROVIDER, request(Vec::new()), &ProviderCx::default(), sink)
            .await
            .unwrap_err();

        assert_eq!(error.kind, ProviderErrorKind::Auth);
        assert_eq!(sent(&endpoint).await.len(), 0);
    }

    async fn a_version_floor_the_api_names_is_met_on_the_retry(transport) {
        let server = MockServer::start().await;
        let floor = ResponseTemplate::new(400).set_body_json(json!({
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": "Claude Code 2.1.1 does not support this model; version 99.0.1 or newer is required.",
                "details": { "error_code": "claude_code_version_too_old" }
            }
        }));
        Mock::given(method("GET")).and(path("/v1/models")).respond_with(
            ResponseTemplate::new(200).set_body_raw(MODELS.as_bytes().to_vec(), "application/json"),
        ).mount(&server).await;
        Mock::given(method("POST")).and(path("/v1/messages")).and(header("user-agent", "claude-cli/2.1.310")).respond_with(floor).mount(&server).await;
        Mock::given(method("POST")).and(path("/v1/messages")).and(header("user-agent", "claude-cli/99.0.1")).respond_with(sse(TEXT)).mount(&server).await;
        let endpoint = Endpoint { plugin: initialized(transport, &server).await, server };

        let (_, first) = stream(&endpoint, request(Vec::new())).await;
        let (_, second) = stream(&endpoint, request(Vec::new())).await;

        let refused = first.unwrap_err();
        assert!(refused.retryable, "{refused:?}");
        second.unwrap();
        let second_request = &sent(&endpoint).await[1];
        assert_eq!(second_request["headers"]["user-agent"], "claude-cli/99.0.1");
        let billing = second_request["body"]["system"][0]["text"].as_str().unwrap();
        assert!(billing.contains("cc_version=99.0.1."), "{billing}");
    }
}
