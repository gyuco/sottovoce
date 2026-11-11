use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("tokenizer: {0}")]
    Tokenizer(String),
    #[error("modello di embedding: {0}")]
    Model(String),
}
