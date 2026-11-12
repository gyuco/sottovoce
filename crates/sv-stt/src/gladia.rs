//! Motore Gladia (live v2, modello Solaria) via WebSocket, regione EU.
//!
//! Ogni connessione crea una sessione con `POST /v2/live` (chiave
//! nell'header `x-gladia-key`) che risponde con l'URL WebSocket già
//! autenticato. I messaggi `transcript` portano frasi intere: `is_final`
//! falso = ipotesi, vero = frase definitiva. Lingua automatica: le cinque
//! lingue dell'app con `code_switching`, lingua per frase.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
use sv_audio::Channel;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::StatusCode;

use crate::engine::{Language, SttEngine, SttStream};
use crate::error::SttError;
use crate::event::{Segment, SttEventKind};
use crate::ws::{self, Protocol, Provider};

const INIT_URL: &str = "https://api.gladia.io/v2/live?region=eu-west";
pub const MODEL: &str = "solaria-1";

#[derive(Debug, Clone)]
pub struct GladiaConfig {
    pub api_key: String,
    /// Override dell'URL di creazione sessione (test/proxy).
    pub init_url: Option<String>,
}

pub struct Gladia {
    config: Arc<GladiaProvider>,
}

pub(crate) struct GladiaProvider {
    config: GladiaConfig,
    http: reqwest::Client,
}

impl Gladia {
    pub fn new(config: GladiaConfig) -> Result<Self, SttError> {
        if config.api_key.trim().is_empty() {
            return Err(SttError::MissingApiKey("gladia"));
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| SttError::Connect(e.to_string()))?;
        Ok(Self {
            config: Arc::new(GladiaProvider { config, http }),
        })
    }
}

impl SttEngine for Gladia {
    fn name(&self) -> &'static str {
        "gladia"
    }

    fn start(&self, channel: Channel, language: Language) -> SttStream {
        let config = self.config.clone();
        SttStream::spawn(channel, move |audio, events| {
            ws::run(config, language, channel, audio, events)
        })
    }
}

/// Corpo di `POST /v2/live`.
fn init_body(language: Language) -> serde_json::Value {
    let languages: Vec<&str> = match language {
        Language::Multi => Language::CONCRETE.iter().map(|l| l.code()).collect(),
        fixed => vec![fixed.code()],
    };
    json!({
        "encoding": "wav/pcm",
        "bit_depth": 16,
        "sample_rate": 16000,
        "channels": 1,
        "model": MODEL,
        // Silenzio che chiude una frase (default 0,05 s: troppo spezzettato).
        "endpointing": 0.3,
        "maximum_duration_without_endpointing": 15,
        "language_config": {
            "languages": languages,
            "code_switching": language == Language::Multi,
        },
        "messages_config": { "receive_partial_transcripts": true },
    })
}

#[derive(Deserialize)]
struct InitResponse {
    url: String,
}

impl Provider for GladiaProvider {
    type Session = Session;

    fn name(&self) -> &'static str {
        "gladia"
    }

    async fn request(&self, _channel: Channel, language: Language) -> Result<Request, SttError> {
        let url = self.config.init_url.as_deref().unwrap_or(INIT_URL);
        let resp = self
            .http
            .post(url)
            .header("x-gladia-key", self.config.api_key.trim())
            .json(&init_body(language))
            .send()
            .await
            .map_err(|e| SttError::Connect(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| SttError::Connect(e.to_string()))?;
        if !status.is_success() {
            let status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            return Err(ws::http_error(status, &body));
        }
        let init: InitResponse = serde_json::from_str(&body)
            .map_err(|e| SttError::Protocol(format!("risposta Gladia inattesa: {e}")))?;
        // L'URL contiene già il token della sessione.
        ws::ws_request(&init.url, &[])
    }

    fn session(&self, _channel: Channel, language: Language) -> Session {
        Session {
            fixed: (language != Language::Multi).then(|| language.code().to_owned()),
            ..Session::default()
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Transcript {
        data: TranscriptData,
    },
    EndSession {},
    Error {
        #[serde(default)]
        error: Option<serde_json::Value>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct TranscriptData {
    #[serde(default)]
    is_final: bool,
    utterance: Utterance,
}

#[derive(Debug, Deserialize)]
struct Utterance {
    #[serde(default)]
    text: String,
    /// Secondi dall'inizio della sessione.
    start: f64,
    end: f64,
    #[serde(default)]
    confidence: Option<f32>,
    #[serde(default)]
    language: Option<String>,
}

#[derive(Debug, Default)]
pub struct Session {
    fixed: Option<String>,
    /// Ultima ipotesi mostrata (chiusa come `Final` se lo stream cade).
    partial: Option<Segment>,
    finished: bool,
}

impl Protocol for Session {
    fn close(&mut self) -> Option<Message> {
        Some(Message::text(r#"{"type":"stop_recording"}"#))
    }

    fn on_text(&mut self, text: &str, origin: Duration) -> Result<Vec<SttEventKind>, SttError> {
        let msg = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!("gladia: messaggio non riconosciuto ({e}): {text}");
                return Ok(Vec::new());
            }
        };
        Ok(match msg {
            ServerMessage::Transcript { data } => {
                let u = data.utterance;
                let text = u.text.trim();
                let seg = Segment {
                    text: text.to_owned(),
                    start: origin + ws::secs(u.start),
                    end: origin + ws::secs(u.end),
                    confidence: u.confidence,
                    language: self
                        .fixed
                        .clone()
                        .or_else(|| u.language.as_deref().map(ws::base_code)),
                    ..Segment::default()
                };
                if data.is_final {
                    self.partial = None;
                    if text.is_empty() {
                        vec![SttEventKind::Partial(Segment::default())]
                    } else {
                        vec![SttEventKind::Final(seg)]
                    }
                } else if text.is_empty() {
                    Vec::new()
                } else {
                    self.partial = Some(seg.clone());
                    vec![SttEventKind::Partial(seg)]
                }
            }
            ServerMessage::EndSession {} => {
                self.finished = true;
                Vec::new()
            }
            ServerMessage::Error { error } => {
                tracing::warn!("gladia: errore dal server: {error:?}");
                Vec::new()
            }
            ServerMessage::Other => Vec::new(),
        })
    }

    fn finished(&self) -> bool {
        self.finished
    }

    fn flush(&mut self) -> Option<SttEventKind> {
        self.partial.take().map(SttEventKind::Final)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const O: Duration = Duration::from_secs(2);

    fn transcript(is_final: bool, text: &str, lang: &str) -> String {
        format!(
            r#"{{"session_id":"s","created_at":"2026-01-01T00:00:00Z","type":"transcript",
            "data":{{"id":"u1","is_final":{is_final},"utterance":{{"text":" {text}",
            "start":1.5,"end":3.0,"confidence":0.87,"channel":0,"language":"{lang}",
            "words":[]}}}}}}"#
        )
    }

    #[test]
    fn partial_and_final_utterances() {
        let mut s = Session::default();
        let out = s.on_text(&transcript(false, "ciao a", "it"), O).unwrap();
        assert!(matches!(&out[..], [SttEventKind::Partial(p)] if p.text == "ciao a"));
        let out = s
            .on_text(&transcript(true, "Ciao a tutti.", "it"), O)
            .unwrap();
        let [SttEventKind::Final(f)] = &out[..] else {
            panic!("{out:?}")
        };
        assert_eq!(f.text, "Ciao a tutti.");
        assert_eq!(f.start, O + Duration::from_millis(1500));
        assert_eq!(f.end, O + Duration::from_secs(3));
        assert_eq!(f.language.as_deref(), Some("it"));
        assert_eq!(s.flush(), None);
    }

    #[test]
    fn dropped_connection_keeps_last_partial() {
        let mut s = Session {
            fixed: Some("en".into()),
            ..Session::default()
        };
        s.on_text(&transcript(false, "hello", "es"), O).unwrap();
        let Some(SttEventKind::Final(f)) = s.flush() else {
            panic!()
        };
        assert_eq!(f.language.as_deref(), Some("en"));
        s.on_text(r#"{"type":"end_session","session_id":"s"}"#, O)
            .unwrap();
        assert!(s.finished());
    }

    #[test]
    fn init_body_languages() {
        let multi = init_body(Language::Multi);
        assert_eq!(multi["language_config"]["code_switching"], true);
        assert_eq!(
            multi["language_config"]["languages"],
            json!(["it", "en", "es", "fr", "de"])
        );
        let it = init_body(Language::It);
        assert_eq!(it["language_config"]["languages"], json!(["it"]));
        assert_eq!(it["language_config"]["code_switching"], false);
        assert_eq!(it["encoding"], "wav/pcm");
        assert_eq!(it["messages_config"]["receive_partial_transcripts"], true);
    }
}
