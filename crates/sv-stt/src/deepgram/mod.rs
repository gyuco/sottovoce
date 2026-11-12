//! Motore Deepgram (Nova-3) via WebSocket streaming.
//!
//! Connessione, riconnessione e invio audio stanno in [`crate::ws`]; qui
//! solo URL, autenticazione e messaggi di Deepgram.

mod protocol;

use std::sync::Arc;
use std::time::Duration;

use sv_audio::Channel;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::client::Request;

use crate::engine::{Language, SttEngine, SttStream};
use crate::error::SttError;
use crate::event::SttEventKind;
use crate::ws::{self, Protocol, Provider};
use protocol::{ServerMessage, Utterance};

const DEFAULT_ENDPOINT: &str = "wss://api.deepgram.com/v1/listen";
/// Deepgram chiude dopo ~10 s senza dati: KeepAlive se non inviamo da 5 s.
const KEEPALIVE_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct DeepgramConfig {
    pub api_key: String,
    pub model: String,
    /// Override per test/proxy; default `wss://api.deepgram.com/v1/listen`.
    pub endpoint: Option<String>,
    /// Diarization di Deepgram sul canale "Loro" (parlante per parola).
    pub diarize: bool,
}

pub struct Deepgram {
    config: Arc<DeepgramConfig>,
}

impl Deepgram {
    pub fn new(config: DeepgramConfig) -> Result<Self, SttError> {
        if config.api_key.trim().is_empty() {
            return Err(SttError::MissingApiKey("deepgram"));
        }
        // rustls richiede un crypto provider di processo; errore = già installato.
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            config: Arc::new(config),
        })
    }
}

impl SttEngine for Deepgram {
    fn name(&self) -> &'static str {
        "deepgram"
    }

    fn start(&self, channel: Channel, language: Language) -> SttStream {
        let config = self.config.clone();
        SttStream::spawn(channel, move |audio, events| {
            ws::run(config, language, channel, audio, events)
        })
    }
}

impl Provider for DeepgramConfig {
    type Session = Session;

    fn name(&self) -> &'static str {
        "deepgram"
    }

    async fn request(&self, channel: Channel, language: Language) -> Result<Request, SttError> {
        let diarize = self.diarize && channel == Channel::Them;
        ws::ws_request(
            &listen_url(self, language, diarize),
            &[("Authorization", format!("Token {}", self.api_key.trim()))],
        )
    }

    fn session(&self, _channel: Channel, _language: Language) -> Session {
        Session::default()
    }
}

#[derive(Default)]
pub struct Session {
    utt: Utterance,
}

impl Protocol for Session {
    fn keepalive(&self) -> Option<(Duration, Message)> {
        Some((KEEPALIVE_AFTER, Message::text(r#"{"type":"KeepAlive"}"#)))
    }

    fn close(&mut self) -> Option<Message> {
        Some(Message::text(r#"{"type":"CloseStream"}"#))
    }

    fn on_text(&mut self, text: &str, origin: Duration) -> Result<Vec<SttEventKind>, SttError> {
        Ok(match protocol::parse(text) {
            Ok(ServerMessage::Results(r)) => self.utt.on_results(&r, origin),
            Ok(ServerMessage::UtteranceEnd {}) => self.utt.flush().into_iter().collect(),
            Ok(_) => Vec::new(),
            Err(e) => {
                tracing::debug!("deepgram: messaggio non riconosciuto ({e}): {text}");
                Vec::new()
            }
        })
    }

    fn flush(&mut self) -> Option<SttEventKind> {
        self.utt.flush()
    }
}

fn listen_url(config: &DeepgramConfig, language: Language, diarize: bool) -> String {
    let endpoint = config.endpoint.as_deref().unwrap_or(DEFAULT_ENDPOINT);
    // In multi Nova-3 sceglie tra ~55 lingue e confonde italiano/spagnolo:
    // gli hint orientano il riconoscimento senza escludere le altre lingue.
    let hints: String = if language == Language::Multi {
        Language::CONCRETE
            .iter()
            .map(|l| format!("&language_hint={}", l.code()))
            .collect()
    } else {
        String::new()
    };
    format!(
        "{endpoint}?model={}&language={}&encoding=linear16&sample_rate=16000&channels=1\
         &interim_results=true&smart_format=true&punctuate=true\
         &endpointing=300&utterance_end_ms=1000&vad_events=false{hints}{}",
        config.model,
        language.code(),
        if diarize { "&diarize=true" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_contains_model_and_language() {
        let cfg = DeepgramConfig {
            api_key: "SECRET".into(),
            model: "nova-3".into(),
            endpoint: None,
            diarize: false,
        };
        let url = listen_url(&cfg, Language::Multi, false);
        assert!(url.starts_with("wss://api.deepgram.com/v1/listen?model=nova-3&language=multi&"));
        assert!(url.contains("interim_results=true"));
        assert!(url.contains(
            "&language_hint=it&language_hint=en&language_hint=es&language_hint=fr&language_hint=de"
        ));
        let mono = listen_url(&cfg, Language::It, false);
        assert!(mono.contains("language=it&") && !mono.contains("language_hint"));
        assert!(!url.contains("diarize"));
        assert!(listen_url(&cfg, Language::It, true).ends_with("&diarize=true"));
        assert!(!url.contains("SECRET"), "la chiave va solo nell'header");
    }

    #[test]
    fn empty_key_is_rejected() {
        let cfg = DeepgramConfig {
            api_key: "  ".into(),
            model: "nova-3".into(),
            endpoint: None,
            diarize: false,
        };
        assert!(matches!(
            Deepgram::new(cfg),
            Err(SttError::MissingApiKey("deepgram"))
        ));
    }
}
