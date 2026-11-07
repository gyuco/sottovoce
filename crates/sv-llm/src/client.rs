use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::types::{ChatRequest, ChatResponse, ModelInfo, Usage};
use crate::{BoxFuture, LlmError, LlmProvider};

/// Timeout di default per richiesta (retry inclusi).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Attesa massima fra due tentativi, anche se `Retry-After` chiede di più.
const MAX_RETRY_DELAY: Duration = Duration::from_secs(5);
/// Caratteri del corpo di errore conservati in [`LlmError::Http`].
const ERROR_BODY_MAX: usize = 500;

/// Retry su 429 e 5xx con backoff esponenziale. Mai su altri 4xx.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base_delay: Duration::from_millis(500),
        }
    }
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
    /// Modelli che hanno rifiutato `response_format`: per loro si salta
    /// subito al JSON via prompt.
    no_json_mode: Mutex<HashSet<String>>,
}

impl OpenAiCompatible {
    /// `base_url` include la versione, es. `http://localhost:11434/v1`.
    pub fn new(base_url: impl Into<String>) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Self {
            http: reqwest::Client::builder()
                .user_agent(concat!("sottovoce/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_default(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key: None,
            headers: reqwest::header::HeaderMap::new(),
            timeout: DEFAULT_TIMEOUT,
            retry: RetryPolicy::default(),
            no_json_mode: Mutex::default(),
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
        let mut json_mode = req.json && !self.json_mode_disabled(&req.model);
        let mut attempts = 0;

        loop {
            let body = request_body(&req, json_mode);
            let (status, text, n) = self
                .send_with_retry(|| {
                    self.authed(self.http.post(self.url("chat/completions")))
                        .json(&body)
                })
                .await?;
            attempts += n;

            if status.is_success() {
                return parse_chat(&text, started.elapsed(), attempts);
            }
            if json_mode && status == StatusCode::BAD_REQUEST {
                tracing::info!(model = %req.model, "response_format rifiutato, ripiego su JSON via prompt");
                self.disable_json_mode(&req.model);
                json_mode = false;
                continue;
            }
            return Err(status_error(status, text));
        }
    }

    async fn list_models_inner(&self) -> Result<Vec<ModelInfo>, LlmError> {
        let (status, text, _) = self
            .send_with_retry(|| self.authed(self.http.get(self.url("models"))))
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
                chat_completions: m
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

    /// Invia la richiesta ritentando su 429/5xx. Ritorna l'ultimo stato
    /// ricevuto (non necessariamente di successo), il corpo e i tentativi.
    async fn send_with_retry(
        &self,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> Result<(StatusCode, String, u32), LlmError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let resp = build().send().await.map_err(LlmError::from_reqwest)?;
            let status = resp.status();
            let retry_after = retry_after(&resp);
            let text = resp.text().await.map_err(LlmError::from_reqwest)?;

            let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            if !retryable || attempt > self.retry.max_retries {
                return Ok((status, text, attempt));
            }
            let backoff = self.retry.base_delay * 2u32.pow(attempt - 1);
            let delay = retry_after
                .unwrap_or_default()
                .max(backoff)
                .min(MAX_RETRY_DELAY);
            tracing::debug!(%status, attempt, ?delay, "richiesta LLM ritentata");
            tokio::time::sleep(delay).await;
        }
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

    fn json_mode_disabled(&self, model: &str) -> bool {
        self.no_json_mode.lock().is_ok_and(|s| s.contains(model))
    }

    fn disable_json_mode(&self, model: &str) {
        if let Ok(mut s) = self.no_json_mode.lock() {
            s.insert(model.to_owned());
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

fn request_body(req: &ChatRequest, json_mode: bool) -> Value {
    let mut body = json!({
        "model": req.model,
        "messages": req.messages,
        "stream": false,
    });
    if let Some(t) = req.temperature {
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

fn status_error(status: StatusCode, body: String) -> LlmError {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => LlmError::Unauthorized,
        StatusCode::TOO_MANY_REQUESTS => LlmError::RateLimited,
        _ => LlmError::Http {
            status: status.as_u16(),
            body: truncate(body),
        },
    }
}

fn truncate(mut s: String) -> String {
    if let Some((i, _)) = s.char_indices().nth(ERROR_BODY_MAX) {
        s.truncate(i);
        s.push('…');
    }
    s
}

fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let secs: u64 = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs))
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
