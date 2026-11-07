//! Client nativo per la Messages API di Anthropic (`POST /v1/messages`).
//!
//! Non esiste un SDK Rust ufficiale: HTTP diretto, come da documentazione
//! (header `x-api-key` e `anthropic-version`). Differenze dal protocollo
//! OpenAI gestite qui:
//! - i messaggi `system` vanno nel campo `system` di primo livello;
//! - `max_tokens` è obbligatorio;
//! - con la versione API `2026-10-21` `temperature` non è accettata: non
//!   viene inviata;
//! - niente `response_format`: il JSON si chiede nel prompt (e si legge con
//!   [`crate::parse_json`]);
//! - `output_config.effort` (default `low`, per la latenza); se il modello
//!   non lo supporta (400) lo si omette per le chiamate successive.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::http::{DEFAULT_TIMEOUT, RetryPolicy, http_client, send_with_retry, status_error};
use crate::types::{ChatRequest, ChatResponse, ModelInfo, Role, Usage};
use crate::{BoxFuture, LlmError, LlmProvider};

pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/v1";
const API_VERSION: &str = "2026-10-21";
/// `max_tokens` se la richiesta non lo indica (risposte brevi, non streaming).
const DEFAULT_MAX_TOKENS: u32 = 4096;

pub struct Anthropic {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    timeout: Duration,
    retry: RetryPolicy,
    effort: Option<String>,
    /// Modelli che hanno rifiutato `output_config.effort`.
    no_effort: Mutex<HashSet<String>>,
}

impl Anthropic {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            http: http_client(),
            base_url: ANTHROPIC_BASE_URL.to_owned(),
            api_key: api_key.into(),
            timeout: DEFAULT_TIMEOUT,
            retry: RetryPolicy::default(),
            effort: Some("low".to_owned()),
            no_effort: Mutex::default(),
        }
    }

    /// Per i test (server finto).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_owned();
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

    /// `low` | `medium` | `high` | …; `None` = default del modello.
    pub fn with_effort(mut self, effort: Option<&str>) -> Self {
        self.effort = effort.map(str::to_owned);
        self
    }

    fn request(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        rb.header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
    }

    async fn chat_inner(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        let started = Instant::now();
        let mut attempts = 0;
        loop {
            let effort = self
                .effort
                .as_deref()
                .filter(|_| !self.no_effort.lock().is_ok_and(|s| s.contains(&req.model)));
            let body = request_body(&req, effort);
            let url = format!("{}/messages", self.base_url);
            let (status, text, n) = send_with_retry(self.retry, || {
                self.request(self.http.post(&url)).json(&body)
            })
            .await?;
            attempts += n;

            if status.is_success() {
                return parse_message(&text, started.elapsed(), attempts);
            }
            if status == StatusCode::BAD_REQUEST
                && effort.is_some()
                && text.to_lowercase().contains("effort")
            {
                tracing::info!(model = %req.model, "effort rifiutato, lo ometto");
                if let Ok(mut s) = self.no_effort.lock() {
                    s.insert(req.model.clone());
                }
                continue;
            }
            return Err(status_error(status, text));
        }
    }

    async fn list_models_inner(&self) -> Result<Vec<ModelInfo>, LlmError> {
        let url = format!("{}/models?limit=1000", self.base_url);
        let (status, text, _) =
            send_with_retry(self.retry, || self.request(self.http.get(&url))).await?;
        if !status.is_success() {
            return Err(status_error(status, text));
        }
        let list: ModelList =
            serde_json::from_str(&text).map_err(|e| LlmError::InvalidJson(e.to_string()))?;
        let mut models: Vec<ModelInfo> = list
            .data
            .into_iter()
            .map(|m| ModelInfo {
                id: m.id,
                name: m.display_name,
                usable: true,
            })
            .collect();
        models.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(models)
    }
}

impl LlmProvider for Anthropic {
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

fn request_body(req: &ChatRequest, effort: Option<&str>) -> Value {
    let mut system: Vec<&str> = Vec::new();
    let mut messages = Vec::new();
    for m in &req.messages {
        match m.role {
            Role::System => system.push(&m.content),
            Role::User => messages.push(json!({ "role": "user", "content": m.content })),
            Role::Assistant => {
                messages.push(json!({ "role": "assistant", "content": m.content }));
            }
        }
    }
    let mut system = system.join("\n\n");
    if req.json {
        system.push_str("\n\nRespond with a single JSON object only, with no other text.");
    }
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        "messages": messages,
    });
    let system = system.trim();
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    if let Some(effort) = effort {
        body["output_config"] = json!({ "effort": effort });
    }
    body
}

fn parse_message(text: &str, latency: Duration, attempts: u32) -> Result<ChatResponse, LlmError> {
    let resp: MessageResponse =
        serde_json::from_str(text).map_err(|e| LlmError::InvalidJson(e.to_string()))?;
    if resp.stop_reason.as_deref() == Some("refusal") {
        return Err(LlmError::Refused);
    }
    let content: String = resp
        .content
        .iter()
        .filter(|b| b.kind == "text")
        .filter_map(|b| b.text.as_deref())
        .collect();
    if content.trim().is_empty() {
        return Err(LlmError::InvalidJson("risposta senza contenuto".into()));
    }
    let usage = resp.usage.map(|u| Usage {
        prompt_tokens: u.input_tokens,
        completion_tokens: u.output_tokens,
        total_tokens: u.input_tokens + u.output_tokens,
    });
    Ok(ChatResponse {
        content,
        model: resp.model,
        usage,
        latency,
        attempts,
    })
}

#[derive(Deserialize)]
struct MessageResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    model: Option<String>,
    stop_reason: Option<String>,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
    display_name: Option<String>,
}
