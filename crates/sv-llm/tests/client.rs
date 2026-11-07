use std::time::Duration;

use serde_json::{Value, json};
use sv_llm::{ChatRequest, LlmError, LlmProvider, Message, OpenAiCompatible, RetryPolicy};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn completion(content: &str) -> Value {
    json!({
        "id": "x",
        "model": "test-model",
        "choices": [{ "index": 0, "message": { "role": "assistant", "content": content } }],
        "usage": { "prompt_tokens": 12, "completion_tokens": 3, "total_tokens": 15 }
    })
}

fn client(server: &MockServer) -> OpenAiCompatible {
    OpenAiCompatible::new(format!("{}/v1/", server.uri())).with_retry(RetryPolicy {
        max_retries: 2,
        base_delay: Duration::from_millis(10),
    })
}

fn request() -> ChatRequest {
    ChatRequest::new(
        "test-model",
        vec![Message::system("sys"), Message::user("ciao")],
    )
}

fn has_response_format(req: &Request) -> bool {
    serde_json::from_slice::<Value>(&req.body).is_ok_and(|v| v.get("response_format").is_some())
}

#[tokio::test]
async fn chat_ok_with_auth_and_metrics() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .and(body_partial_json(json!({
            "model": "test-model",
            "temperature": 0.5,
            "max_tokens": 50,
            "messages": [{ "role": "system", "content": "sys" }, { "role": "user", "content": "ciao" }]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion("hello")))
        .expect(1)
        .mount(&server)
        .await;

    let resp = client(&server)
        .with_api_key(Some("sk-test".into()))
        .chat(request().temperature(0.5).max_tokens(50))
        .await
        .unwrap();

    assert_eq!(resp.content, "hello");
    assert_eq!(resp.model.as_deref(), Some("test-model"));
    assert_eq!(resp.usage.unwrap().total_tokens, 15);
    assert_eq!(resp.attempts, 1);
}

#[tokio::test]
async fn no_auth_header_without_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(|req: &Request| !req.headers.contains_key("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion("ok")))
        .expect(1)
        .mount(&server)
        .await;

    let resp = client(&server)
        .with_api_key(Some("  ".into()))
        .chat(request())
        .await
        .unwrap();
    assert_eq!(resp.content, "ok");
}

#[tokio::test]
async fn extra_headers_on_every_request() {
    let server = MockServer::start().await;
    Mock::given(header("x-opencode-session", "s-1"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion("ok")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(header("x-opencode-session", "s-1"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .expect(1)
        .mount(&server)
        .await;

    let llm = client(&server)
        .with_header("x-opencode-session", "s-1")
        .with_header("bad name", "x");
    llm.chat(request()).await.unwrap();
    llm.list_models().await.unwrap();
}

#[tokio::test]
async fn json_mode_sends_response_format() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(
            json!({ "response_format": { "type": "json_object" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion(r#"{"a":1}"#)))
        .expect(1)
        .mount(&server)
        .await;

    let resp = client(&server).chat(request().json()).await.unwrap();
    let v: Value = sv_llm::parse_json(&resp.content).unwrap();
    assert_eq!(v["a"], 1);
}

#[tokio::test]
async fn json_mode_falls_back_on_400_and_remembers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(has_response_format)
        .respond_with(ResponseTemplate::new(400).set_body_string("response_format not supported"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(|req: &Request| !has_response_format(req))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(completion("```json\n{\"a\":2}\n```")),
        )
        .expect(2)
        .mount(&server)
        .await;

    let llm = client(&server);
    let resp = llm.chat(request().json()).await.unwrap();
    assert_eq!(resp.attempts, 2);
    let v: Value = sv_llm::parse_json(&resp.content).unwrap();
    assert_eq!(v["a"], 2);

    // Seconda chiamata: niente response_format, un solo tentativo.
    let resp = llm.chat(request().json()).await.unwrap();
    assert_eq!(resp.attempts, 1);
}

#[tokio::test]
async fn bad_request_without_json_mode_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad model"))
        .expect(1)
        .mount(&server)
        .await;

    let err = client(&server).chat(request()).await.unwrap_err();
    assert!(matches!(err, LlmError::Http { status: 400, ref body } if body == "bad model"));
}

#[tokio::test]
async fn retries_on_429_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion("ok")))
        .expect(1)
        .mount(&server)
        .await;

    let resp = client(&server).chat(request()).await.unwrap();
    assert_eq!(resp.content, "ok");
    assert_eq!(resp.attempts, 2);
}

#[tokio::test]
async fn gives_up_after_max_retries() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_string("down"))
        .expect(3)
        .mount(&server)
        .await;

    let err = client(&server).chat(request()).await.unwrap_err();
    assert!(matches!(err, LlmError::Http { status: 503, .. }));
}

#[tokio::test]
async fn rate_limited_after_max_retries() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429))
        .expect(3)
        .mount(&server)
        .await;

    let err = client(&server).chat(request()).await.unwrap_err();
    assert!(matches!(err, LlmError::RateLimited));
}

#[tokio::test]
async fn unauthorized_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("invalid key"))
        .expect(1)
        .mount(&server)
        .await;

    let err = client(&server).chat(request()).await.unwrap_err();
    assert!(matches!(err, LlmError::Unauthorized));
}

#[tokio::test]
async fn timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(completion("tardi"))
                .set_delay(Duration::from_millis(500)),
        )
        .mount(&server)
        .await;

    let err = client(&server)
        .chat(request().timeout(Duration::from_millis(100)))
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Timeout));
}

#[tokio::test]
async fn missing_content_is_invalid_json() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "choices": [] })))
        .mount(&server)
        .await;

    let err = client(&server).chat(request()).await.unwrap_err();
    assert!(matches!(err, LlmError::InvalidJson(_)));
}

#[tokio::test]
async fn connection_refused() {
    let err = OpenAiCompatible::new("http://127.0.0.1:9/v1")
        .chat(request())
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Connect(_)));
}

#[tokio::test]
async fn list_models_sorted() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{ "id": "qwen3:8b" }, { "id": "gemma3:4b" }]
        })))
        .mount(&server)
        .await;

    let models = client(&server).list_models().await.unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["gemma3:4b", "qwen3:8b"]);
    assert!(models.iter().all(|m| m.chat_completions));
}

#[tokio::test]
async fn list_models_flags_opencode_api() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                { "id": "glm-5.3", "name": "GLM 5.3", "api": { "chat-completions": {} } },
                { "id": "claude-haiku-5-5", "api": { "messages": {} } },
                { "id": "gpt-6-luna", "api": { "responses": {} } }
            ]
        })))
        .mount(&server)
        .await;

    let models = client(&server).list_models().await.unwrap();
    let chat: Vec<&str> = models
        .iter()
        .filter(|m| m.chat_completions)
        .map(|m| m.id.as_str())
        .collect();
    assert_eq!(chat, ["glm-5.3"]);
    assert_eq!(models[1].name.as_deref(), Some("GLM 5.3"));
}
