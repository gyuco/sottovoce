//! Motore Deepgram (Nova-3) via WebSocket streaming.
//!
//! Una connessione per canale. Audio PCM s16le 16 kHz mono. Riconnessione
//! automatica con backoff; errori di autenticazione non vengono ritentati.

mod protocol;

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use sv_audio::{AudioFrame, Channel};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::engine::{EventSender, Language, SttEngine, SttStream};
use crate::error::SttError;
use crate::event::SttEventKind;
use protocol::{ServerMessage, Utterance};

const DEFAULT_ENDPOINT: &str = "wss://api.deepgram.com/v1/listen";
/// Deepgram chiude dopo ~10 s senza dati: KeepAlive se non inviamo da 5 s.
const KEEPALIVE_AFTER: Duration = Duration::from_secs(5);
/// Attesa degli ultimi risultati dopo `CloseStream`.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RETRIES: u32 = 3;

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
    config: DeepgramConfig,
}

impl Deepgram {
    pub fn new(config: DeepgramConfig) -> Result<Self, SttError> {
        if config.api_key.trim().is_empty() {
            return Err(SttError::MissingApiKey("deepgram"));
        }
        // rustls richiede un crypto provider di processo; errore = già installato.
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self { config })
    }
}

impl SttEngine for Deepgram {
    fn name(&self) -> &'static str {
        "deepgram"
    }

    fn start(&self, channel: Channel, language: Language) -> SttStream {
        let config = self.config.clone();
        SttStream::spawn(channel, move |audio, events| {
            run(config, language, channel, audio, events)
        })
    }
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

enum Outcome {
    /// Audio finito e stream chiuso pulito.
    Ended,
    /// Il consumatore degli eventi non c'è più.
    ConsumerGone,
    /// Connessione persa: si può ritentare.
    Lost(String),
}

async fn run(
    config: DeepgramConfig,
    language: Language,
    channel: Channel,
    mut audio: mpsc::Receiver<AudioFrame>,
    events: EventSender,
) {
    let mut attempt = 0u32;
    loop {
        let diarize = config.diarize && channel == Channel::Them;
        let outcome = match connect(&config, language, diarize).await {
            Ok(ws) => {
                tracing::info!(?channel, "deepgram connesso");
                attempt = 0;
                if !events.send(SttEventKind::Connected).await {
                    return;
                }
                session(ws, &mut audio, &events).await
            }
            Err(e @ SttError::Unauthorized) | Err(e @ SttError::Protocol(_)) => {
                events
                    .send(SttEventKind::Error {
                        message: e.to_string(),
                    })
                    .await;
                return;
            }
            Err(e) => Outcome::Lost(e.to_string()),
        };
        match outcome {
            Outcome::Ended => {
                events.send(SttEventKind::Disconnected).await;
                return;
            }
            Outcome::ConsumerGone => return,
            Outcome::Lost(reason) => {
                attempt += 1;
                tracing::warn!(?channel, attempt, "deepgram: connessione persa: {reason}");
                if attempt > MAX_RETRIES {
                    events
                        .send(SttEventKind::Error {
                            message: format!("connessione STT persa: {reason}"),
                        })
                        .await;
                    return;
                }
                if !events.send(SttEventKind::Reconnecting { attempt }).await {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500 << (attempt - 1))).await;
                // L'audio accumulato durante il backoff è vecchio: lo scartiamo.
                while audio.try_recv().is_ok() {}
                if audio.is_closed() && audio.is_empty() {
                    events.send(SttEventKind::Disconnected).await;
                    return;
                }
            }
        }
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

async fn connect(
    config: &DeepgramConfig,
    language: Language,
    diarize: bool,
) -> Result<Ws, SttError> {
    let mut req = listen_url(config, language, diarize)
        .into_client_request()
        .map_err(|e| SttError::Protocol(e.to_string()))?;
    let auth = HeaderValue::from_str(&format!("Token {}", config.api_key.trim()))
        .map_err(|_| SttError::Protocol("API key con caratteri non validi".into()))?;
    req.headers_mut().insert("Authorization", auth);

    match tokio_tungstenite::connect_async(req).await {
        Ok((ws, _)) => Ok(ws),
        Err(WsError::Http(resp)) => {
            let status = resp.status();
            let body = resp
                .body()
                .as_deref()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            Err(match status {
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => SttError::Unauthorized,
                s if s.is_client_error() => SttError::Protocol(format!("HTTP {s}: {body}")),
                s => SttError::Connect(format!("HTTP {s}")),
            })
        }
        Err(e) => Err(SttError::Connect(e.to_string())),
    }
}

async fn session(ws: Ws, audio: &mut mpsc::Receiver<AudioFrame>, events: &EventSender) -> Outcome {
    let (mut sink, mut stream) = ws.split();
    let mut utt = Utterance::default();
    // Tempo di cattura del primo frame inviato: origine dei tempi Deepgram.
    let mut origin: Option<Duration> = None;
    let mut last_sent = Instant::now();
    let mut keepalive = tokio::time::interval(Duration::from_secs(1));

    let outcome = loop {
        tokio::select! {
            frame = audio.recv() => match frame {
                Some(f) => {
                    origin.get_or_insert(f.start);
                    if let Err(e) = sink.send(Message::Binary(pcm16(&f.samples).into())).await {
                        break Outcome::Lost(e.to_string());
                    }
                    last_sent = Instant::now();
                }
                None => {
                    let _ = sink.send(Message::text(r#"{"type":"CloseStream"}"#)).await;
                    let drain = async {
                        while let Some(Ok(msg)) = stream.next().await {
                            if let Message::Text(t) = msg
                                && !handle(&t, &mut utt, origin, events).await
                            {
                                return Outcome::ConsumerGone;
                            }
                        }
                        Outcome::Ended
                    };
                    break tokio::time::timeout(CLOSE_TIMEOUT, drain)
                        .await
                        .unwrap_or(Outcome::Ended);
                }
            },
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    if !handle(&t, &mut utt, origin, events).await {
                        break Outcome::ConsumerGone;
                    }
                }
                Some(Ok(Message::Close(frame))) => {
                    let reason = frame
                        .map(|f| format!("{} {}", u16::from(f.code), f.reason))
                        .unwrap_or_else(|| "chiusura dal server".into());
                    break Outcome::Lost(reason);
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => break Outcome::Lost(e.to_string()),
                None => break Outcome::Lost("stream terminato".into()),
            },
            _ = keepalive.tick() => {
                if last_sent.elapsed() >= KEEPALIVE_AFTER {
                    if let Err(e) = sink.send(Message::text(r#"{"type":"KeepAlive"}"#)).await {
                        break Outcome::Lost(e.to_string());
                    }
                    last_sent = Instant::now();
                }
            }
        }
    };

    // Frase rimasta aperta (fine audio o connessione persa): la chiudiamo.
    if let Some(kind) = utt.flush()
        && !events.send(kind).await
    {
        return Outcome::ConsumerGone;
    }
    outcome
}

/// Gestisce un messaggio testuale. Ritorna false se il consumatore è sparito.
async fn handle(
    text: &str,
    utt: &mut Utterance,
    origin: Option<Duration>,
    events: &EventSender,
) -> bool {
    let kinds = match protocol::parse(text) {
        Ok(ServerMessage::Results(r)) => utt.on_results(&r, origin.unwrap_or_default()),
        Ok(ServerMessage::UtteranceEnd {}) => utt.flush().into_iter().collect(),
        Ok(_) => Vec::new(),
        Err(e) => {
            tracing::debug!("deepgram: messaggio non riconosciuto ({e}): {text}");
            Vec::new()
        }
    };
    for kind in kinds {
        if !events.send(kind).await {
            return false;
        }
    }
    true
}

/// f32 [-1, 1] → PCM s16 little-endian.
fn pcm16(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm16_converts_and_clamps() {
        assert_eq!(
            pcm16(&[0.0, 1.0, -1.0, 2.0]),
            [0, 0, 0xff, 0x7f, 0x01, 0x80, 0xff, 0x7f]
        );
    }

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
