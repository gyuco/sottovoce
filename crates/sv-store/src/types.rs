use std::collections::BTreeMap;

use serde::Serialize;

/// Dati con cui nasce un meeting (prima frase).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeetingDraft {
    pub name: String,
    pub goal: String,
    pub notes: String,
    /// Provider STT effettivo (`deepgram`, `local`).
    pub stt_engine: String,
    /// Modello STT effettivo (`nova-3`, `parakeet-tdt-0.6b-v3`...).
    pub stt_model: String,
    /// Lingua del canale "Io".
    pub my_language: String,
    /// Lingua del canale "Loro".
    pub their_language: String,
    /// Nomi dati dall'utente ai parlanti (0 = "Speaker 1").
    pub speakers: BTreeMap<u8, String>,
}

/// Frase finale da salvare. `session_id` è l'id del `Final` nella sessione.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSegment {
    pub session_id: u64,
    /// `me` (microfono) o `them` (audio di sistema).
    pub channel: String,
    pub speaker: Option<u8>,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// Lingua rilevata dal motore, se disponibile.
    pub language: Option<String>,
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    pub id: i64,
    pub name: String,
    pub goal: String,
    pub notes: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub stt_engine: String,
    pub stt_model: String,
    pub my_language: String,
    pub their_language: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: i64,
    pub meeting_id: i64,
    pub session_id: u64,
    pub channel: String,
    pub speaker: Option<u8>,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub language: Option<String>,
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Translation {
    pub id: i64,
    pub segment_id: i64,
    pub session_id: u64,
    pub text: String,
    pub language: String,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerName {
    pub speaker: u8,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Assist {
    pub id: i64,
    pub kind: String,
    pub text: String,
    pub at_ms: u64,
}

/// Tutto ciò che serve a mostrare un meeting salvato.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetails {
    pub meeting: Meeting,
    pub segments: Vec<Segment>,
    pub translations: Vec<Translation>,
    pub speakers: Vec<SpeakerName>,
    pub assists: Vec<Assist>,
}

/// Riga dello storico: meeting + numero di frasi.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    #[serde(flatten)]
    pub meeting: Meeting,
    pub segment_count: u32,
}

/// Pagina dello storico, più recenti prima.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingPage {
    pub meetings: Vec<MeetingSummary>,
    pub total: u32,
}
