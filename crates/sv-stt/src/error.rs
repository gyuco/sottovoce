#[derive(Debug, thiserror::Error)]
pub enum SttError {
    #[error("API key mancante per il provider {0}")]
    MissingApiKey(&'static str),
    #[error("API key non valida o non autorizzata")]
    Unauthorized,
    #[error("connessione al servizio STT fallita: {0}")]
    Connect(String),
    #[error("errore di protocollo STT: {0}")]
    Protocol(String),
}
