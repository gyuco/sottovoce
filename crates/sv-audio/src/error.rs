use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("nessun dispositivo di input disponibile")]
    NoInputDevice,
    #[error("dispositivo di input non trovato: {0}")]
    DeviceNotFound(String),
    #[error("formato campioni non supportato: {0}")]
    UnsupportedFormat(String),
    #[error("errore cpal: {0}")]
    Cpal(#[from] cpal::Error),
    #[error("errore Core Audio in {call}: OSStatus {status}")]
    CoreAudio { call: &'static str, status: i32 },
    #[error("cattura audio di sistema non supportata su questa piattaforma")]
    SystemCaptureUnsupported,
    #[error("errore resampler: {0}")]
    Resampler(String),
    #[error("thread di cattura terminato prima dell'avvio")]
    WorkerDied,
}
