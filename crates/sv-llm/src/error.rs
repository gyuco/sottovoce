#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("API key mancante o non autorizzata")]
    Unauthorized,
    #[error("limite di richieste superato")]
    RateLimited,
    #[error("timeout della richiesta LLM")]
    Timeout,
    #[error("connessione al servizio LLM fallita: {0}")]
    Connect(String),
    #[error("errore HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("risposta non valida: {0}")]
    InvalidJson(String),
    #[error("il modello ha rifiutato la richiesta")]
    Refused,
}

impl LlmError {
    pub(crate) fn from_reqwest(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            Self::Timeout
        } else if err.is_decode() {
            Self::InvalidJson(err.to_string())
        } else {
            Self::Connect(err.to_string())
        }
    }
}
