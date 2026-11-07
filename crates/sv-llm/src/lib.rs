//! LLM per Sottovoce.
//!
//! Il trait [`LlmProvider`] astrae il modello linguistico. Implementazioni:
//! - [`OpenAiCompatible`]: protocollo `/chat/completions`; copre Ollama,
//!   OpenCode Zen/Go (solo i modelli chat completions), OpenAI e qualunque
//!   endpoint compatibile;
//! - [`Anthropic`]: Messages API nativa di Anthropic.

mod anthropic;
mod client;
mod error;
mod http;
mod json;
mod types;

pub use anthropic::{ANTHROPIC_BASE_URL, Anthropic};
pub use client::OpenAiCompatible;
pub use error::LlmError;
pub use http::RetryPolicy;
pub use json::parse_json;
pub use types::{ChatRequest, ChatResponse, Message, ModelInfo, Role, Usage};

use std::future::Future;
use std::pin::Pin;

/// Future in box, necessaria per usare il trait come `dyn LlmProvider`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Modello linguistico usato da traduzione, live assist e riepiloghi.
pub trait LlmProvider: Send + Sync {
    /// Una chiamata di chat completa (niente streaming).
    fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<ChatResponse, LlmError>>;

    /// Modelli disponibili, in ordine alfabetico di id.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>, LlmError>>;
}
