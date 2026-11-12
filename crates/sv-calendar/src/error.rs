#[derive(Debug, thiserror::Error)]
pub enum CalendarError {
    #[error("errore di rete: {0}")]
    Http(#[from] reqwest::Error),
    #[error("errore locale: {0}")]
    Io(#[from] std::io::Error),
    #[error("accesso negato dall'utente ({0})")]
    Denied(String),
    #[error(
        "accesso al calendario non concesso: spunta il permesso sul calendario nella pagina di Google"
    )]
    MissingScope,
    #[error("tempo scaduto in attesa del consenso nel browser")]
    Timeout,
    #[error("collegamento annullato")]
    Cancelled,
    #[error("accesso revocato o scaduto: ricollega il calendario")]
    Revoked,
    #[error("client OAuth non valido o configurato male: {0}")]
    InvalidClient(String),
    #[error("risposta del calendario non valida: {0}")]
    Invalid(String),
    #[error("errore del calendario ({status}): {message}")]
    Api { status: u16, message: String },
}
