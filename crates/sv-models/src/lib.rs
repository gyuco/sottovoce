//! Modelli locali di Sottovoce (VAD, STT, embedding del parlante e del testo).
//!
//! - [`catalog`]: modelli noti, con URL fissati a un commit, dimensioni e SHA-256.
//! - [`ModelStore`]: stato su disco, download con resume e verifica, eliminazione.

mod catalog;
mod error;
mod store;

pub use catalog::{
    CAMPLUS, EMBEDDING_GEMMA_2, ModelFile, ModelKind, ModelSpec, PARAKEET_V3, SILERO_VAD, catalog,
    find,
};
pub use error::ModelError;
pub use store::{DownloadProgress, ModelStatus, ModelStore};
