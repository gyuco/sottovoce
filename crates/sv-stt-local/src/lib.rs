//! Trascrizione locale per Sottovoce: niente audio a terzi, funziona offline.
//!
//! Pipeline per canale: `AudioFrame` → VAD (`sv-vad`, una frase alla volta)
//! → riconoscitore offline (Parakeet v3 via `sherpa-onnx`) →
//! `SttEvent::Final`. Mentre il VAD sente parlato il canale mostra un
//! parziale "…". Il modello è caricato una volta e condiviso fra i canali.

mod engine;
pub mod lang;
mod recognizer;

use std::path::PathBuf;

pub use engine::{LocalEngine, LocalEngineConfig, SPEAKING_PLACEHOLDER};
pub use recognizer::{Decoded, LocalModel, Recognizer};

#[derive(Debug, Clone, thiserror::Error)]
pub enum LocalSttError {
    #[error("file del modello mancante: {0}")]
    MissingFile(PathBuf),
    #[error("modello STT locale non disponibile: {0}")]
    Load(String),
}
