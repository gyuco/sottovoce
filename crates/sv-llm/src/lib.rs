//! LLM per Sottovoce.
//!
//! Il trait [`LlmProvider`] astrae il modello linguistico. L'unica
//! implementazione, [`OpenAiCompatible`], parla il protocollo
//! `/chat/completions` di OpenAI e copre Ollama, OpenCode Zen (solo i modelli
//! serviti come chat completions) e qualunque endpoint compatibile.

mod client;
mod error;
mod json;
mod types;

pub use client::{OpenAiCompatible, RetryPolicy};
pub use error::LlmError;
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
