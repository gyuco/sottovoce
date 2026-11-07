use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// Richiesta di chat.
#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Chiede un oggetto JSON (`response_format: json_object`). Se il
    /// provider non lo supporta si ripiega sul solo prompt: il risultato va
    /// comunque letto con [`crate::parse_json`].
    pub json: bool,
    /// Timeout di questa richiesta; `None` = default del client.
    pub timeout: Option<Duration>,
}

impl ChatRequest {
    pub fn new(model: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            model: model.into(),
            messages,
            ..Self::default()
        }
    }

    pub fn temperature(mut self, t: f32) -> Self {
        self.temperature = Some(t);
        self
    }

    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = Some(n);
        self
    }

    pub fn json(mut self) -> Self {
        self.json = true;
        self
    }

    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = Some(d);
        self
    }
}

/// Modello disponibile presso il provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    /// Nome leggibile, se il provider lo fornisce.
    pub name: Option<String>,
    /// Servito su `/chat/completions`, quindi usabile da questo client.
    pub chat_completions: bool,
}

/// Token consumati, se il provider li riporta.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
}

/// Risposta di chat con le metriche della chiamata.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: String,
    /// Modello che ha risposto, se riportato.
    pub model: Option<String>,
    pub usage: Option<Usage>,
    /// Tempo totale, retry e fallback inclusi.
    pub latency: Duration,
    /// Richieste HTTP inviate (1 = nessun retry né fallback).
    pub attempts: u32,
}
