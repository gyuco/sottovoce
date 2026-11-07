use std::time::Duration;

use serde_json::{Value, json};
use sv_llm::{Anthropic, ChatRequest, LlmError, LlmProvider, Message, RetryPolicy};
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn client(server: &MockServer) -> Anthropic {
    Anthropic::new("sk-ant-test")
        .with_base_url(format!("{}/v1", server.uri()))
        .with_retry(RetryPolicy {
            max_retries: 2,
            base_delay: Duration::from_millis(10),
        })
}

fn message(text: &str, stop_reason: &str) -> Value {
    json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-5-5",
        "content": [
            { "type": "thinking", "thinking": "" },
            { "type": "text", "text": text }
        ],
        "stop_reason": stop_reason,
        "usage": { "input_tokens": 20, "output_tokens": 4 }
    })
}

fn body(req: &Request) -> Value {
    serde_json::from_slice(&req.body).unwrap()
}

#[tokio::test]
async fn messages_request_shape_and_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-test"))
        .and(header("anthropic-version", "2026-10-21"))
        .and(body_partial_json(json!({
            "model": "claude-haiku-5-5",
            "max_tokens": 4096,
            "system": "Translate.",
            "messages": [{ "role": "user", "content": "hello" }],
            "output_config": { "effort": "low" }
        })))
        .and(|req: &Request| body(req).get("temperature").is_none())
        .respond_with(ResponseTemplate::new(200).set_body_json(message("ciao", "end_turn")))
        .expect(1)
        .mount(&server)
        .await;

    let resp = client(&server)
        .chat(
            ChatRequest::new(
                "claude-haiku-5-5",
                vec![Message::system("Translate."), Message::user("hello")],
            )
            .temperature(0.2),
        )
        .await
        .unwrap();
    assert_eq!(resp.content, "ciao");
    assert_eq!(resp.usage.unwrap().total_tokens, 24);
}

#[tokio::test]
async fn json_request_asks_for_json_in_system() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(|req: &Request| {
            body(req)["system"].as_str().is_some_and(|s| {
                s.starts_with("Coach.") && s.ends_with("JSON object only, with no other text.")
            })
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(message("{\"a\":1}", "end_turn")))
        .expect(1)
        .mount(&server)
        .await;

    let resp = client(&server)
        .chat(ChatRequest::new("m", vec![Message::system("Coach."), Message::user("x")]).json())
        .await
        .unwrap();
    let v: Value = sv_llm::parse_json(&resp.content).unwrap();
    assert_eq!(v["a"], 1);
}

#[tokio::test]
async fn effort_rejected_is_dropped_and_remembered() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(|req: &Request| body(req).get("output_config").is_some())
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"effort is not supported on this model"}}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(|req: &Request| body(req).get("output_config").is_none())
        .respond_with(ResponseTemplate::new(200).set_body_json(message("ok", "end_turn")))
        .expect(2)
        .mount(&server)
        .await;

    let llm = client(&server);
    let req = || ChatRequest::new("claude-haiku-4-5", vec![Message::user("x")]);
    assert_eq!(llm.chat(req()).await.unwrap().attempts, 2);
    assert_eq!(llm.chat(req()).await.unwrap().attempts, 1);
}

#[tokio::test]
async fn refusal_and_auth_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(|req: &Request| body(req)["model"] == "refuse")
        .respond_with(ResponseTemplate::new(200).set_body_json(message("", "refusal")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(|req: &Request| body(req)["model"] == "badkey")
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;

    let llm = client(&server);
    let err = llm
        .chat(ChatRequest::new("refuse", vec![Message::user("x")]))
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Refused));
    let err = llm
        .chat(ChatRequest::new("badkey", vec![Message::user("x")]))
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Unauthorized));
}

#[tokio::test]
async fn list_models_with_api_key_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(query_param("limit", "1000"))
        .and(header("x-api-key", "sk-ant-test"))
        .and(header("anthropic-version", "2026-10-21"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                { "id": "claude-sonnet-5-5", "display_name": "Claude Sonnet 5.5" },
                { "id": "claude-haiku-5-5", "display_name": "Claude Haiku 5.5" }
            ],
            "has_more": false
        })))
        .mount(&server)
        .await;

    let models = client(&server).list_models().await.unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["claude-haiku-5-5", "claude-sonnet-5-5"]);
    assert!(models.iter().all(|m| m.usable));
    assert_eq!(models[0].name.as_deref(), Some("Claude Haiku 5.5"));
}
