use std::time::Duration;

use serde::Serialize;
use sv_audio::Channel;

/// Porzione di testo riconosciuta, con tempi relativi all'avvio della cattura.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    /// Id progressivo della frase nella sessione, assegnato dall'app ai
    /// `Final` (0 = non assegnato, es. per i parziali).
    pub id: u64,
    pub text: String,
    #[serde(serialize_with = "ser_ms")]
    pub start: Duration,
    #[serde(serialize_with = "ser_ms")]
    pub end: Duration,
    /// Confidenza 0..1, se fornita dal motore.
    pub confidence: Option<f32>,
    /// Lingua rilevata (codice BCP-47, es. `en`), se fornita dal motore.
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SttEventKind {
    /// Connessione al motore stabilita (o ristabilita).
    Connected,
    /// Ipotesi provvisoria: sostituisce il parziale precedente del canale.
    Partial(Segment),
    /// Testo definitivo: chiude il parziale corrente del canale.
    Final(Segment),
    /// Connessione persa, il motore sta ritentando.
    Reconnecting { attempt: u32 },
    /// Errore non recuperabile: lo stream termina.
    Error { message: String },
    /// Stream chiuso normalmente.
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SttEvent {
    pub channel: Channel,
    #[serde(flatten)]
    pub kind: SttEventKind,
}

fn ser_ms<S: serde::Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_u64(d.as_millis() as u64)
}
