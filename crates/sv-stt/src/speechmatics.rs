//! Motore Speechmatics (Real-Time API v2) via WebSocket, endpoint EU.
//!
//! Dopo la connessione si manda `StartRecognition` e si aspetta
//! `RecognitionStarted` prima dell'audio. `AddPartialTranscript` è
//! l'ipotesi della porzione in corso, `AddTranscript` la porzione definitiva:
//! le porzioni definitive si uniscono finché `EndOfUtterance` (o un punto di
//! fine frase) non chiude la frase. Lingua fissa: modello `enhanced` con
//! fine frase dopo 0,7 s di silenzio. Automatica: modello `melia-1` con
//! `language: "multi"` e lingua per parola (non accetta la fine frase per
//! silenzio, quindi la frase si chiude sui punti).

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
use sv_audio::Channel;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::client::Request;

use crate::engine::{Language, SttEngine, SttStream};
use crate::error::SttError;
use crate::event::{Segment, SttEventKind};
use crate::ws::{self, LanguageTally, Protocol, Provider};

const ENDPOINT: &str = "wss://eu.rt.speechmatics.com/v2";
pub const MODEL_FIXED: &str = "enhanced";
pub const MODEL_MULTI: &str = "melia-1";

#[derive(Debug, Clone)]
pub struct SpeechmaticsConfig {
    pub api_key: String,
    /// Override (es. `wss://global.rt.speechmatics.com/v2`).
    pub endpoint: Option<String>,
}

pub struct Speechmatics {
    config: Arc<SpeechmaticsConfig>,
}

impl Speechmatics {
    pub fn new(config: SpeechmaticsConfig) -> Result<Self, SttError> {
        if config.api_key.trim().is_empty() {
            return Err(SttError::MissingApiKey("speechmatics"));
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            config: Arc::new(config),
        })
    }
}

impl SttEngine for Speechmatics {
    fn name(&self) -> &'static str {
        "speechmatics"
    }

    fn start(&self, channel: Channel, language: Language) -> SttStream {
        let config = self.config.clone();
        SttStream::spawn(channel, move |audio, events| {
            ws::run(config, language, channel, audio, events)
        })
    }
}

impl Provider for SpeechmaticsConfig {
    type Session = Session;

    fn name(&self) -> &'static str {
        "speechmatics"
    }

    async fn request(&self, _channel: Channel, _language: Language) -> Result<Request, SttError> {
        ws::ws_request(
            self.endpoint.as_deref().unwrap_or(ENDPOINT),
            &[("Authorization", format!("Bearer {}", self.api_key.trim()))],
        )
    }

    fn session(&self, _channel: Channel, language: Language) -> Session {
        Session::new(language)
    }
}

/// Messaggio `StartRecognition` per la lingua scelta.
fn start_recognition(language: Language) -> serde_json::Value {
    let transcription = if language == Language::Multi {
        let hints: Vec<&str> = Language::CONCRETE.iter().map(|l| l.code()).collect();
        json!({
            "language": "multi",
            "language_hints": hints,
            "model": MODEL_MULTI,
            "enable_partials": true,
            "max_delay": 1.0,
        })
    } else {
        json!({
            "language": language.code(),
            "model": MODEL_FIXED,
            "enable_partials": true,
            "max_delay": 1.0,
            "conversation_config": { "end_of_utterance_silence_trigger": 0.7 },
        })
    };
    json!({
        "message": "StartRecognition",
        "audio_format": { "type": "raw", "encoding": "pcm_s16le", "sample_rate": 16000 },
        "transcription_config": transcription,
    })
}

#[derive(Debug, Deserialize)]
#[serde(tag = "message")]
enum ServerMessage {
    RecognitionStarted {},
    AddPartialTranscript(Transcript),
    AddTranscript(Transcript),
    EndOfUtterance {},
    EndOfTranscript {},
    Error {
        #[serde(default, rename = "type")]
        kind: String,
        #[serde(default)]
        reason: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct Transcript {
    metadata: Metadata,
    #[serde(default)]
    results: Vec<Item>,
}

#[derive(Debug, Deserialize)]
struct Metadata {
    #[serde(default)]
    transcript: String,
    start_time: f64,
    end_time: f64,
}

#[derive(Debug, Deserialize)]
struct Item {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    is_eos: bool,
    #[serde(default)]
    alternatives: Vec<ItemAlternative>,
}

#[derive(Debug, Deserialize)]
struct ItemAlternative {
    #[serde(default)]
    confidence: Option<f32>,
    #[serde(default)]
    language: Option<String>,
}

pub struct Session {
    language: Language,
    started: bool,
    finished: bool,
    /// Blocchi audio inviati (per `EndOfStream`).
    seq: u64,
    text: String,
    start: Option<Duration>,
    end: Duration,
    confidence: Option<f32>,
    languages: LanguageTally,
}

impl Session {
    fn new(language: Language) -> Self {
        Self {
            language,
            started: false,
            finished: false,
            seq: 0,
            text: String::new(),
            start: None,
            end: Duration::ZERO,
            confidence: None,
            languages: LanguageTally::default(),
        }
    }

    fn sentence_language(&self) -> Option<String> {
        match self.language {
            Language::Multi => self.languages.top(),
            fixed => Some(fixed.code().to_owned()),
        }
    }

    fn segment(&self, text: String, end: Duration) -> Segment {
        Segment {
            text,
            start: self.start.unwrap_or(end),
            end,
            confidence: self.confidence,
            language: self.sentence_language(),
            ..Segment::default()
        }
    }

    fn on_final(&mut self, t: Transcript, origin: Duration) -> Vec<SttEventKind> {
        let piece = t.metadata.transcript.trim();
        if piece.is_empty() {
            return Vec::new();
        }
        self.start
            .get_or_insert(origin + ws::secs(t.metadata.start_time));
        self.end = origin + ws::secs(t.metadata.end_time);
        self.text = ws::join(&self.text, piece);
        for item in &t.results {
            let Some(alt) = item.alternatives.first() else {
                continue;
            };
            if item.kind == "word" {
                self.confidence = ws::min_conf(self.confidence, alt.confidence);
                if let Some(l) = &alt.language {
                    self.languages.add(l, 1);
                }
            }
        }
        // Senza fine frase per silenzio (modello multilingue) si chiude sui punti.
        let eos = t.results.last().is_some_and(|i| i.is_eos);
        if self.language == Language::Multi && eos {
            return self.flush().into_iter().collect();
        }
        vec![SttEventKind::Partial(
            self.segment(self.text.clone(), self.end),
        )]
    }
}

impl Protocol for Session {
    fn on_open(&mut self) -> Vec<Message> {
        vec![Message::text(start_recognition(self.language).to_string())]
    }

    fn ready(&self) -> bool {
        self.started
    }

    fn audio(&mut self, pcm: Vec<u8>) -> Message {
        self.seq += 1;
        Message::Binary(pcm.into())
    }

    fn close(&mut self) -> Option<Message> {
        if !self.started {
            self.finished = true;
            return None;
        }
        Some(Message::text(
            json!({ "message": "EndOfStream", "last_seq_no": self.seq }).to_string(),
        ))
    }

    fn on_text(&mut self, text: &str, origin: Duration) -> Result<Vec<SttEventKind>, SttError> {
        let msg = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!("speechmatics: messaggio non riconosciuto ({e}): {text}");
                return Ok(Vec::new());
            }
        };
        Ok(match msg {
            ServerMessage::RecognitionStarted {} => {
                self.started = true;
                Vec::new()
            }
            ServerMessage::AddPartialTranscript(t) => {
                let piece = t.metadata.transcript.trim();
                if piece.is_empty() && self.text.is_empty() {
                    return Ok(Vec::new());
                }
                if self.start.is_none() && !piece.is_empty() {
                    self.start = Some(origin + ws::secs(t.metadata.start_time));
                }
                let end = origin + ws::secs(t.metadata.end_time);
                vec![SttEventKind::Partial(
                    self.segment(ws::join(&self.text, piece), end.max(self.end)),
                )]
            }
            ServerMessage::AddTranscript(t) => self.on_final(t, origin),
            ServerMessage::EndOfUtterance {} => self.flush().into_iter().collect(),
            ServerMessage::EndOfTranscript {} => {
                self.finished = true;
                Vec::new()
            }
            ServerMessage::Error { kind, reason } => {
                return Err(match kind.as_str() {
                    "not_authorised" => SttError::Unauthorized,
                    _ => SttError::Protocol(format!("Speechmatics: {kind} {reason}")),
                });
            }
            ServerMessage::Other => Vec::new(),
        })
    }

    fn finished(&self) -> bool {
        self.finished
    }

    fn flush(&mut self) -> Option<SttEventKind> {
        if self.text.is_empty() {
            self.start = None;
            return None;
        }
        let text = std::mem::take(&mut self.text);
        let seg = self.segment(text, self.end);
        self.start = None;
        self.confidence = None;
        self.languages = LanguageTally::default();
        Some(SttEventKind::Final(seg))
    }

    fn fatal_close(&self, code: u16) -> bool {
        // not_authorised, not_allowed, invalid_model, quota_exceeded, timelimit.
        matches!(code, 4001 | 4003 | 4004 | 4005 | 4006)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const O: Duration = Duration::from_secs(5);

    fn transcript(kind: &str, text: &str, start: f64, end: f64, eos: bool, lang: &str) -> String {
        format!(
            r#"{{"message":"{kind}","format":"2.9","metadata":{{"transcript":"{text}",
            "start_time":{start},"end_time":{end}}},"results":[
            {{"type":"word","start_time":{start},"end_time":{end},
              "alternatives":[{{"content":"x","confidence":0.9,"language":"{lang}"}}]}},
            {{"type":"punctuation","start_time":{end},"end_time":{end},"is_eos":{eos},
              "alternatives":[{{"content":".","confidence":1.0}}]}}]}}"#
        )
    }

    #[test]
    fn waits_for_start_then_builds_sentence() {
        let mut s = Session::new(Language::It);
        assert!(!s.ready());
        let open = s.on_open();
        let Message::Text(start) = &open[0] else {
            panic!()
        };
        let v: serde_json::Value = serde_json::from_str(start).unwrap();
        assert_eq!(v["transcription_config"]["language"], "it");
        assert_eq!(v["transcription_config"]["model"], "enhanced");
        assert_eq!(v["audio_format"]["encoding"], "pcm_s16le");

        s.on_text(r#"{"message":"RecognitionStarted","id":"x"}"#, O)
            .unwrap();
        assert!(s.ready());

        let out = s
            .on_text(
                &transcript("AddPartialTranscript", "ciao a", 0.5, 1.0, false, "it"),
                O,
            )
            .unwrap();
        assert!(matches!(&out[..], [SttEventKind::Partial(p)] if p.text == "ciao a"));
        s.on_text(
            &transcript("AddTranscript", "ciao a tutti", 0.5, 1.5, false, "it"),
            O,
        )
        .unwrap();
        s.on_text(
            &transcript("AddTranscript", "come state?", 1.6, 2.5, false, "it"),
            O,
        )
        .unwrap();
        let out = s
            .on_text(r#"{"message":"EndOfUtterance","metadata":{}}"#, O)
            .unwrap();
        let [SttEventKind::Final(f)] = &out[..] else {
            panic!("{out:?}")
        };
        assert_eq!(f.text, "ciao a tutti come state?");
        assert_eq!(f.start, O + Duration::from_millis(500));
        assert_eq!(f.end, O + Duration::from_millis(2500));
        assert_eq!(f.language.as_deref(), Some("it"));
        assert_eq!(s.flush(), None);
    }

    #[test]
    fn multi_closes_on_end_of_sentence_with_word_language() {
        let mut s = Session::new(Language::Multi);
        let Message::Text(start) = &s.on_open()[0] else {
            panic!()
        };
        let v: serde_json::Value = serde_json::from_str(start).unwrap();
        assert_eq!(v["transcription_config"]["language"], "multi");
        assert_eq!(v["transcription_config"]["model"], "melia-1");
        assert!(v["transcription_config"]["conversation_config"].is_null());

        let out = s
            .on_text(
                &transcript("AddTranscript", "we ship it.", 0.0, 1.0, true, "en"),
                O,
            )
            .unwrap();
        assert!(
            matches!(&out[..], [SttEventKind::Final(f)] if f.language.as_deref() == Some("en"))
        );
    }

    #[test]
    fn end_of_stream_counts_chunks_and_errors_are_fatal() {
        let mut s = Session::new(Language::En);
        s.on_text(r#"{"message":"RecognitionStarted"}"#, O).unwrap();
        s.audio(vec![0; 4]);
        s.audio(vec![0; 4]);
        let Some(Message::Text(end)) = s.close() else {
            panic!()
        };
        assert!(end.contains(r#""last_seq_no":2"#), "{end}");
        assert!(matches!(
            s.on_text(
                r#"{"message":"Error","type":"not_authorised","reason":"bad"}"#,
                O
            ),
            Err(SttError::Unauthorized)
        ));
        assert!(matches!(
            s.on_text(
                r#"{"message":"Error","type":"quota_exceeded","reason":"x"}"#,
                O
            ),
            Err(SttError::Protocol(_))
        ));
        s.on_text(r#"{"message":"EndOfTranscript"}"#, O).unwrap();
        assert!(s.finished());
    }
}
