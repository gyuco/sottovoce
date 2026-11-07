use crate::store::ModelStatus;

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("modello sconosciuto: {0}")]
    Unknown(String),
    #[error("modello {id} non pronto ({status:?}): scaricalo nelle Impostazioni → Modelli locali")]
    NotReady { id: String, status: ModelStatus },
    #[error("errore di rete: {0}")]
    Network(String),
    #[error("il server ha risposto {status}")]
    Http { status: u16 },
    #[error("{file}: contenuto diverso da quello atteso (SHA-256)")]
    Checksum { file: String },
    #[error("download annullato")]
    Cancelled,
    #[error("errore su disco: {0}")]
    Io(#[from] std::io::Error),
}
