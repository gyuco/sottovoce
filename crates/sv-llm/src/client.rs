use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::http::{DEFAULT_TIMEOUT, RetryPolicy, http_client, send_with_retry, status_error};
use crate::types::{ChatRequest, ChatResponse, ModelInfo, Usage};
use crate::{BoxFuture, LlmError, LlmProvider};

/// Parametri che un modello ha rifiutato con un 400: per lui non si mandano
/// più (scoperto alla prima chiamata, valido per la vita del client).
#[derive(Debug, Clone, Copy, Default)]
struct Quirks {
    /// `response_format` non supportato: JSON solo via prompt.
    no_json_mode: bool,
    /// `temperature` non supportata (es. modelli di ragionamento OpenAI).
    no_temperature: bool,
}

/// Client per endpoint OpenAI-compatible (`{base_url}/chat/completions`).
pub struct OpenAiCompatible {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    /// Header aggiunti a ogni richiesta (es. `x-opencode-session`).
    headers: reqwest::header::HeaderMap,
    timeout: Duration,
    retry: RetryPolicy,
    quirks: Mutex<HashMap<String, Quirks>>,
}

impl OpenAiCompatible {
    /// `base_url` include la versione, es. `http://localhost:11434/v1`.
    pub fn new(base_url: impl Into<String>) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Self {
            http: http_client(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key: None,
            headers: reqwest::header::HeaderMap::new(),
            timeout: DEFAULT_TIMEOUT,
            retry: RetryPolicy::default(),
            quirks: Mutex::default(),
        }
    }

    /// Chiave per `Authorization: Bearer`; vuota o `None` = nessuna.
    pub fn with_api_key(mut self, key: Option<String>) -> Self {
        self.api_key = key.filter(|k| !k.trim().is_empty());
        self
    }

    /// Header extra su ogni richiesta. Nome o valore non validi: ignorato
    /// con un warning.
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        use reqwest::header::{HeaderName, HeaderValue};
        match (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            (Ok(n), Ok(v)) => {
                self.headers.insert(n, v);
            }
            _ => tracing::warn!(name, "header LLM non valido, ignorato"),
        }
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    async fn chat_inner(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        let started = Instant::now();
        let mut attempts = 0;

        loop {
            let quirks = self.quirks(&req.model);
            let json_mode = req.json && !quirks.no_json_mode;
            let temperature = req.temperature.filter(|_| !quirks.no_temperature);
            let body = request_body(&req, json_mode, temperature);
            let (status, text, n) = send_with_retry(self.retry, || {
                self.authed(self.http.post(self.url("chat/completions")))
                    .json(&body)
            })
            .await?;
            attempts += n;

            if status.is_success() {
                return parse_chat(&text, started.elapsed(), attempts);
            }
            if status == StatusCode::BAD_REQUEST {
                // Un parametro rifiutato: lo si toglie e si ritenta subito.
                if temperature.is_some() && text.to_lowercase().contains("temperature") {
                    tracing::info!(model = %req.model, "temperature rifiutata, la ometto");
                    self.update_quirks(&req.model, |q| q.no_temperature = true);
                    continue;
                }
                if json_mode {
                    tracing::info!(model = %req.model, "response_format rifiutato, ripiego su JSON via prompt");
                    self.update_quirks(&req.model, |q| q.no_json_mode = true);
                    continue;
                }
            }
            return Err(status_error(status, text));
        }
    }

    async fn list_models_inner(&self) -> Result<Vec<ModelInfo>, LlmError> {
        let (status, text, _) = send_with_retry(self.retry, || {
            self.authed(self.http.get(self.url("models")))
        })
        .await?;
        if !status.is_success() {
            return Err(status_error(status, text));
        }
        let list: ModelList =
            serde_json::from_str(&text).map_err(|e| LlmError::InvalidJson(e.to_string()))?;
        let mut models: Vec<ModelInfo> = list
            .data
            .into_iter()
            .map(|m| ModelInfo {
                // Il catalogo OpenCode dichiara le API di ogni modello; gli
                // altri provider non lo fanno: si assume chat completions.
                usable: m
                    .api
                    .as_ref()
                    .and_then(Value::as_object)
                    .is_none_or(|api| api.contains_key("chat-completions")),
                name: m.name,
                id: m.id,
            })
            .collect();
        models.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(models)
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.base_url)
    }

    fn authed(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let rb = rb.headers(self.headers.clone());
        match &self.api_key {
            Some(key) => rb.bearer_auth(key),
            None => rb,
        }
    }

    fn quirks(&self, model: &str) -> Quirks {
        self.quirks
            .lock()
            .map(|q| q.get(model).copied().unwrap_or_default())
            .unwrap_or_default()
    }

    fn update_quirks(&self, model: &str, f: impl FnOnce(&mut Quirks)) {
        if let Ok(mut q) = self.quirks.lock() {
            f(q.entry(model.to_owned()).or_default());
        }
    }
}

impl LlmProvider for OpenAiCompatible {
    fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<ChatResponse, LlmError>> {
        Box::pin(async move {
            let timeout = req.timeout.unwrap_or(self.timeout);
            tokio::time::timeout(timeout, self.chat_inner(req))
                .await
                .map_err(|_| LlmError::Timeout)?
        })
    }

    fn list_models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>, LlmError>> {
        Box::pin(async move {
            tokio::time::timeout(self.timeout, self.list_models_inner())
                .await
                .map_err(|_| LlmError::Timeout)?
        })
    }
}

fn request_body(req: &ChatRequest, json_mode: bool, temperature: Option<f32>) -> Value {
    let mut body = json!({
        "model": req.model,
        "messages": req.messages,
        "stream": false,
    });
    if let Some(t) = temperature {
        body["temperature"] = json!(t);
    }
    if let Some(n) = req.max_tokens {
        body["max_tokens"] = json!(n);
    }
    if json_mode {
        body["response_format"] = json!({ "type": "json_object" });
    }
    body
}

fn parse_chat(text: &str, latency: Duration, attempts: u32) -> Result<ChatResponse, LlmError> {
    let resp: CompletionResponse =
        serde_json::from_str(text).map_err(|e| LlmError::InvalidJson(e.to_string()))?;
    let content = resp
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .ok_or_else(|| LlmError::InvalidJson("risposta senza contenuto".into()))?;
    Ok(ChatResponse {
        content,
        model: resp.model,
        usage: resp.usage,
        latency,
        attempts,
    })
}

#[derive(Deserialize)]
struct CompletionResponse {
    #[serde(default)]
    choices: Vec<Choice>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
    name: Option<String>,
    api: Option<Value>,
}
