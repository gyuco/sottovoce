use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

/// Frequenza di campionamento dei frame pubblicati (input per VAD/STT).
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Canale logico: chi sta parlando.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// Microfono dell'utente ("Io").
    Me,
    /// Audio di sistema: gli altri partecipanti ("Loro").
    Them,
}

/// Blocco di audio 16 kHz mono f32, pronto per VAD/STT.
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub channel: Channel,
    /// Tempo dall'avvio della cattura al primo campione del frame.
    pub start: Duration,
    pub samples: Arc<[f32]>,
}

/// Livello d'ingresso di un canale, calcolato su una finestra di ~50 ms.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelEvent {
    pub channel: Channel,
    /// RMS in dBFS (−100 = silenzio).
    pub rms_db: f32,
    /// Picco in dBFS.
    pub peak_db: f32,
}
