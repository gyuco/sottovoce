//! Motore AssemblyAI (Universal Streaming v3) via WebSocket.
//!
//! Il server manda messaggi `Turn` con la trascrizione del turno in corso:
//! ogni `Turn` sostituisce il precedente finché `end_of_turn` non lo chiude.
//! Con `format_turns=true` dopo il `Turn` di fine turno ne arriva uno con
//! maiuscole e punteggiatura (`turn_is_formatted`): quello è il `Final`.
//! Modello multilingue (en, es, fr, de, it, pt) con rilevamento della lingua
//! in modalità automatica.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use sv_audio::Channel;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::client::Request;

use crate::engine::{Language, SttEngine, SttStream};
use crate::error::SttError;
use crate::event::{Segment, SttEventKind};
use crate::ws::{self, Protocol, Provider};

const ENDPOINT: &str = "wss://streaming.assemblyai.com/v3/ws";
pub const MODEL: &str = "universal-streaming-multilingual";
const KEEPALIVE_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct AssemblyAiConfig {
    pub api_key: String,
    /// Override per test/proxy (es. endpoint EU `wss://streaming.eu.assemblyai.com/v3/ws`).
    pub endpoint: Option<String>,
}

pub struct AssemblyAi {
    config: Arc<AssemblyAiConfig>,
}

impl AssemblyAi {
    pub fn new(config: AssemblyAiConfig) -> Result<Self, SttError> {
        if config.api_key.trim().is_empty() {
            return Err(SttError::MissingApiKey("assemblyai"));
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            config: Arc::new(config),
        })
    }
}

impl SttEngine for AssemblyAi {
    fn name(&self) -> &'static str {
        "assemblyai"
    }

    fn start(&self, channel: Channel, language: Language) -> SttStream {
        let config = self.config.clone();
        SttStream::spawn(channel, move |audio, events| {
            ws::run(config, language, channel, audio, events)
        })
    }
}

fn url(config: &AssemblyAiConfig, language: Language) -> String {
    let endpoint = config.endpoint.as_deref().unwrap_or(ENDPOINT);
    format!(
        "{endpoint}?sample_rate=16000&encoding=pcm_s16le&format_turns=true&speech_model={MODEL}{}",
        if language == Language::Multi {
            "&language_detection=true"
        } else {
            ""
        }
    )
}

impl Provider for AssemblyAiConfig {
    type Session = Session;

    fn name(&self) -> &'static str {
        "assemblyai"
    }

    async fn request(&self, _channel: Channel, language: Language) -> Result<Request, SttError> {
        ws::ws_request(
            &url(self, language),
            &[("Authorization", self.api_key.trim().to_owned())],
        )
    }

    fn session(&self, _channel: Channel, language: Language) -> Session {
        Session {
            fixed: (language != Language::Multi).then(|| language.code().to_owned()),
            ..Session::default()
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ServerMessage {
    Begin {},
    Turn(Turn),
    Termination {},
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct Turn {
    #[serde(default)]
    turn_order: u64,
    #[serde(default)]
    transcript: String,
    #[serde(default)]
    end_of_turn: bool,
    #[serde(default)]
    turn_is_formatted: bool,
    #[serde(default)]
    language_code: Option<String>,
    #[serde(default)]
    words: Vec<Word>,
}

#[derive(Debug, Deserialize)]
struct Word {
    /// Millisecondi dall'inizio dell'audio della connessione.
    start: u64,
    end: u64,
    #[serde(default)]
    confidence: Option<f32>,
}

/// Errore del server (`{"error": "..."}`), fuori dallo schema a `type`.
#[derive(Debug, Deserialize)]
struct ServerError {
    error: String,
}

#[derive(Debug, Default)]
pub struct Session {
    /// Lingua fissata all'avvio, se non automatica.
    fixed: Option<String>,
    /// Turno aperto (ultimo testo ricevuto) e suo numero.
    open: Option<(u64, Segment)>,
    /// Ultimo turno già chiuso: i suoi `Turn` tardivi si ignorano.
    closed: Option<u64>,
    finished: bool,
}

impl Session {
    fn on_turn(&mut self, t: Turn, origin: Duration) -> Vec<SttEventKind> {
        if self.closed.is_some_and(|c| t.turn_order <= c) {
            return Vec::new();
        }
        let text = t.transcript.trim();
        let (start, end) = match (t.words.first(), t.words.last()) {
            (Some(a), Some(b)) => (
                origin + Duration::from_millis(a.start),
                origin + Duration::from_millis(b.end),
            ),
            _ => match &self.open {
                Some((_, s)) => (s.start, s.end),
                None => (origin, origin),
            },
        };
        let confidence = t.words.iter().filter_map(|w| w.confidence).reduce(f32::min);
        let language = self
            .fixed
            .clone()
            .or_else(|| t.language_code.as_deref().map(ws::base_code))
            .or_else(|| self.open.as_ref().and_then(|(_, s)| s.language.clone()));
        let seg = Segment {
            text: text.to_owned(),
            start,
            end,
            confidence,
            language,
            ..Segment::default()
        };
        if t.end_of_turn && t.turn_is_formatted {
            self.open = None;
            self.closed = Some(t.turn_order);
            return if text.is_empty() {
                // Turno senza parole: si toglie il parziale.
                vec![SttEventKind::Partial(Segment {
                    text: String::new(),
                    ..seg
                })]
            } else {
                vec![SttEventKind::Final(seg)]
            };
        }
        if text.is_empty() {
            return Vec::new();
        }
        self.open = Some((t.turn_order, seg.clone()));
        vec![SttEventKind::Partial(seg)]
    }
}

impl Protocol for Session {
    fn keepalive(&self) -> Option<(Duration, Message)> {
        Some((KEEPALIVE_AFTER, Message::text(r#"{"type":"KeepAlive"}"#)))
    }

    fn close(&mut self) -> Option<Message> {
        Some(Message::text(r#"{"type":"Terminate"}"#))
    }

    fn on_text(&mut self, text: &str, origin: Duration) -> Result<Vec<SttEventKind>, SttError> {
        if let Ok(ServerError { error }) = serde_json::from_str(text) {
            return Err(SttError::Protocol(error));
        }
        Ok(match serde_json::from_str(text) {
            Ok(ServerMessage::Turn(t)) => self.on_turn(t, origin),
            Ok(ServerMessage::Termination {}) => {
                self.finished = true;
                Vec::new()
            }
            Ok(_) => Vec::new(),
            Err(e) => {
                tracing::debug!("assemblyai: messaggio non riconosciuto ({e}): {text}");
                Vec::new()
            }
        })
    }

    fn finished(&self) -> bool {
        self.finished
    }

    fn flush(&mut self) -> Option<SttEventKind> {
        let (order, seg) = self.open.take()?;
        self.closed = Some(order);
        Some(SttEventKind::Final(seg))
    }

    fn fatal_close(&self, code: u16) -> bool {
        // 1008 = policy violation (chiave o parametri), 3005 = sessione rifiutata.
        matches!(code, 1008 | 3005)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const O: Duration = Duration::from_secs(10);

    fn turn(order: u64, text: &str, eot: bool, formatted: bool, lang: Option<&str>) -> String {
        let words = if text.is_empty() {
            "[]".to_owned()
        } else {
            r#"[{"text":"a","start":1000,"end":1400,"confidence":0.9,"word_is_final":true},
                {"text":"b","start":1500,"end":2500,"confidence":0.8,"word_is_final":true}]"#
                .to_owned()
        };
        let lang = lang.map_or(String::new(), |l| format!(r#","language_code":"{l}""#));
        format!(
            r#"{{"type":"Turn","turn_order":{order},"turn_is_formatted":{formatted},
            "end_of_turn":{eot},"end_of_turn_confidence":0.7,"transcript":"{text}",
            "words":{words}{lang}}}"#
        )
    }

    #[test]
    fn partial_then_formatted_final() {
        let mut s = Session::default();
        let out = s
            .on_text(&turn(0, "ciao a", false, false, None), O)
            .unwrap();
        assert!(matches!(&out[..], [SttEventKind::Partial(p)] if p.text == "ciao a"));
        let out = s
            .on_text(&turn(0, "ciao a tutti", true, false, None), O)
            .unwrap();
        assert!(matches!(&out[..], [SttEventKind::Partial(_)]));
        let out = s
            .on_text(&turn(0, "Ciao a tutti.", true, true, Some("it")), O)
            .unwrap();
        let [SttEventKind::Final(f)] = &out[..] else {
            panic!("{out:?}")
        };
        assert_eq!(f.text, "Ciao a tutti.");
        assert_eq!(f.start, O + Duration::from_millis(1000));
        assert_eq!(f.end, O + Duration::from_millis(2500));
        assert_eq!(f.language.as_deref(), Some("it"));
        assert_eq!(f.confidence, Some(0.8));
        assert_eq!(s.flush(), None);
        // Turno già chiuso: ignorato.
        assert!(
            s.on_text(&turn(0, "Ciao", true, true, None), O)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn fixed_language_and_flush() {
        let mut s = AssemblyAiConfig {
            api_key: "k".into(),
            endpoint: None,
        }
        .session(Channel::Them, Language::En);
        s.on_text(&turn(3, "hello there", false, false, Some("es")), O)
            .unwrap();
        let Some(SttEventKind::Final(f)) = s.flush() else {
            panic!()
        };
        assert_eq!(f.text, "hello there");
        assert_eq!(f.language.as_deref(), Some("en"));
    }

    #[test]
    fn errors_and_termination() {
        let mut s = Session::default();
        assert!(matches!(
            s.on_text(r#"{"error":"Invalid API key"}"#, O),
            Err(SttError::Protocol(m)) if m.contains("Invalid")
        ));
        assert!(!s.finished());
        s.on_text(
            r#"{"type":"Termination","audio_duration_seconds":3,"session_duration_seconds":4}"#,
            O,
        )
        .unwrap();
        assert!(s.finished());
        assert!(
            s.on_text(r#"{"type":"Begin","id":"x","expires_at":1}"#, O)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn url_has_model_and_detection_only_in_multi() {
        let cfg = AssemblyAiConfig {
            api_key: "SECRET".into(),
            endpoint: None,
        };
        let multi = url(&cfg, Language::Multi);
        assert!(multi.starts_with("wss://streaming.assemblyai.com/v3/ws?sample_rate=16000"));
        assert!(multi.contains("speech_model=universal-streaming-multilingual"));
        assert!(multi.ends_with("&language_detection=true"));
        assert!(!url(&cfg, Language::It).contains("language_detection"));
        assert!(!multi.contains("SECRET"));
    }
}
