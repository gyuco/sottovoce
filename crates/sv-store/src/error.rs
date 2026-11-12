use std::sync::mpsc::RecvTimeoutError;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("errore del database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("errore su disco: {0}")]
    Io(#[from] std::io::Error),
    #[error("database creato da una versione più recente ({found}, supportata {supported})")]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("meeting {0} non trovato")]
    MeetingNotFound(i64),
    #[error("il nome del meeting non può essere vuoto")]
    EmptyName,
    #[error("contatto {0} non trovato")]
    ContactNotFound(i64),
    #[error("il nome del contatto non può essere vuoto")]
    EmptyContactName,
    #[error("in rubrica c'è già un contatto di nome {0}")]
    DuplicateContact(String),
    #[error("indirizzo email non valido: {0}")]
    InvalidEmail(String),
    #[error("riunione programmata {0} non trovata")]
    ScheduledNotFound(i64),
    #[error("riunione programmata non valida: {0}")]
    InvalidScheduled(String),
    #[error("il task di salvataggio non è più attivo")]
    WriterClosed,
    #[error("il task di salvataggio non ha risposto in tempo: {0}")]
    Timeout(#[from] RecvTimeoutError),
}
