//! Ciclo comune dei motori cloud in streaming su WebSocket.
//!
//! Una connessione per canale: l'audio va al server come PCM s16le 16 kHz
//! mono, i messaggi del server diventano [`SttEventKind`]. Riconnessione
//! automatica con backoff; autenticazione e configurazione errate non si
//! ritentano. Ogni provider descrive solo il proprio protocollo
//! ([`Provider`] + [`Protocol`]).

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use sv_audio::{AudioFrame, Channel};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::engine::{EventSender, Language};
use crate::error::SttError;
use crate::event::SttEventKind;

/// Attesa degli ultimi risultati dopo il messaggio di chiusura.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RETRIES: u32 = 3;
/// Audio tenuto da parte finché il server non è pronto (~10 s).
const MAX_PENDING_FRAMES: usize = 160;

/// Provider in streaming: come aprire la connessione e lo stato di una.
pub(crate) trait Provider: Send + Sync + 'static {
    type Session: Protocol;

    fn name(&self) -> &'static str;

    /// Richiesta WebSocket (URL + header). Può fare prima una chiamata HTTP
    /// (es. Gladia crea la sessione con una POST).
    fn request(
        &self,
        channel: Channel,
        language: Language,
    ) -> impl Future<Output = Result<Request, SttError>> + Send;

    /// Stato del protocollo per una nuova connessione.
    fn session(&self, channel: Channel, language: Language) -> Self::Session;
}

/// Stato del protocollo su una connessione.
pub(crate) trait Protocol: Send {
    /// Messaggi da inviare appena connessi (es. configurazione).
    fn on_open(&mut self) -> Vec<Message> {
        Vec::new()
    }

    /// `false` finché il server non conferma l'avvio: l'audio aspetta.
    fn ready(&self) -> bool {
        true
    }

    /// Messaggio con un blocco di audio PCM s16le.
    fn audio(&mut self, pcm: Vec<u8>) -> Message {
        Message::Binary(pcm.into())
    }

    /// Messaggio da inviare se non si manda audio da un po'.
    fn keepalive(&self) -> Option<(Duration, Message)> {
        None
    }

    /// Fine audio: il server finisce di trascrivere e poi chiude.
    fn close(&mut self) -> Option<Message>;

    /// Messaggio di testo del server. `origin` = tempo di cattura del primo
    /// frame inviato su questa connessione (i tempi del server sono relativi
    /// a esso). Un errore `Unauthorized`/`Protocol` chiude lo stream, uno
    /// `Connect` fa riconnettere.
    fn on_text(&mut self, text: &str, origin: Duration) -> Result<Vec<SttEventKind>, SttError>;

    /// `true` se il server ha finito (dopo `close`): si smette di aspettare.
    fn finished(&self) -> bool {
        false
    }

    /// Chiude la frase rimasta aperta (fine audio, connessione persa).
    fn flush(&mut self) -> Option<SttEventKind>;

    /// Codice di chiusura del server che non ha senso ritentare.
    fn fatal_close(&self, _code: u16) -> bool {
        false
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
    /// Errore non recuperabile.
    Fatal(String),
}

/// Corpo del task di uno stream: connessione, sessione, riconnessione.
pub(crate) async fn run<P: Provider>(
    provider: std::sync::Arc<P>,
    language: Language,
    channel: Channel,
    mut audio: mpsc::Receiver<AudioFrame>,
    events: EventSender,
) {
    let name = provider.name();
    let mut attempt = 0u32;
    loop {
        let connected = match provider.request(channel, language).await {
            Ok(req) => connect(req).await,
            Err(e) => Err(e),
        };
        let outcome = match connected {
            Ok(ws) => {
                tracing::info!(?channel, "{name} connesso");
                attempt = 0;
                if !events.send(SttEventKind::Connected).await {
                    return;
                }
                let proto = provider.session(channel, language);
                session(ws, proto, &mut audio, &events).await
            }
            Err(
                e @ (SttError::Unauthorized | SttError::Protocol(_) | SttError::MissingApiKey(_)),
            ) => Outcome::Fatal(e.to_string()),
            Err(e) => Outcome::Lost(e.to_string()),
        };
        match outcome {
            Outcome::Ended => {
                events.send(SttEventKind::Disconnected).await;
                return;
            }
            Outcome::ConsumerGone => return,
            Outcome::Fatal(message) => {
                tracing::warn!(?channel, "{name}: {message}");
                events.send(SttEventKind::Error { message }).await;
                return;
            }
            Outcome::Lost(reason) => {
                attempt += 1;
                tracing::warn!(?channel, attempt, "{name}: connessione persa: {reason}");
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

/// Richiesta WebSocket da URL e header (nome, valore).
pub(crate) fn ws_request(
    url: &str,
    headers: &[(&'static str, String)],
) -> Result<Request, SttError> {
    let mut req = url
        .into_client_request()
        .map_err(|e| SttError::Protocol(e.to_string()))?;
    for (name, value) in headers {
        let value = HeaderValue::from_str(value)
            .map_err(|_| SttError::Protocol("API key con caratteri non validi".into()))?;
        req.headers_mut().insert(*name, value);
    }
    Ok(req)
}

async fn connect(req: Request) -> Result<Ws, SttError> {
    match tokio_tungstenite::connect_async(req).await {
        Ok((ws, _)) => Ok(ws),
        Err(WsError::Http(resp)) => {
            let status = resp.status();
            let body = resp
                .body()
                .as_deref()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            Err(http_error(status, &body))
        }
        Err(e) => Err(SttError::Connect(e.to_string())),
    }
}

/// Stato HTTP di errore → [`SttError`] (401/403 = chiave, 4xx = config).
pub(crate) fn http_error(status: StatusCode, body: &str) -> SttError {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => SttError::Unauthorized,
        StatusCode::TOO_MANY_REQUESTS | StatusCode::REQUEST_TIMEOUT => {
            SttError::Connect(format!("HTTP {status}"))
        }
        s if s.is_client_error() => SttError::Protocol(format!("HTTP {s}: {}", body.trim())),
        s => SttError::Connect(format!("HTTP {s}")),
    }
}

async fn session<S: Protocol>(
    ws: Ws,
    mut proto: S,
    audio: &mut mpsc::Receiver<AudioFrame>,
    events: &EventSender,
) -> Outcome {
    let (mut sink, mut stream) = ws.split();
    for msg in proto.on_open() {
        if let Err(e) = sink.send(msg).await {
            return Outcome::Lost(e.to_string());
        }
    }
    // Tempo di cattura del primo frame inviato: origine dei tempi del server.
    let mut origin: Option<Duration> = None;
    let mut pending: Vec<AudioFrame> = Vec::new();
    let mut last_sent = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let keepalive = proto.keepalive();

    let outcome = 'session: loop {
        tokio::select! {
            frame = audio.recv() => match frame {
                Some(f) => {
                    if !proto.ready() {
                        if pending.len() >= MAX_PENDING_FRAMES {
                            pending.remove(0);
                        }
                        pending.push(f);
                        continue;
                    }
                    for f in pending.drain(..).chain(std::iter::once(f)) {
                        origin.get_or_insert(f.start);
                        let msg = proto.audio(pcm16(&f.samples));
                        if let Err(e) = sink.send(msg).await {
                            break 'session Outcome::Lost(e.to_string());
                        }
                    }
                    last_sent = Instant::now();
                }
                None => {
                    if let Some(msg) = proto.close() {
                        let _ = sink.send(msg).await;
                    }
                    let drain = async {
                        if proto.finished() {
                            return Outcome::Ended;
                        }
                        while let Some(Ok(msg)) = stream.next().await {
                            if let Message::Text(t) = msg {
                                match deliver(&mut proto, &t, origin, events).await {
                                    Ok(true) => {}
                                    Ok(false) => return Outcome::ConsumerGone,
                                    Err(o) => return o,
                                }
                                if proto.finished() {
                                    break;
                                }
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
                    match deliver(&mut proto, &t, origin, events).await {
                        Ok(true) => {}
                        Ok(false) => break Outcome::ConsumerGone,
                        Err(o) => break o,
                    }
                }
                Some(Ok(Message::Close(frame))) => {
                    let (code, reason) = frame
                        .map(|f| (u16::from(f.code), format!("{} {}", u16::from(f.code), f.reason)))
                        .unwrap_or((0, "chiusura dal server".into()));
                    break if proto.fatal_close(code) {
                        Outcome::Fatal(format!("il servizio STT ha chiuso la connessione: {reason}"))
                    } else {
                        Outcome::Lost(reason)
                    };
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => break Outcome::Lost(e.to_string()),
                None => break Outcome::Lost("stream terminato".into()),
            },
            _ = tick.tick() => {
                if let Some((after, msg)) = &keepalive
                    && last_sent.elapsed() >= *after
                {
                    if let Err(e) = sink.send(msg.clone()).await {
                        break Outcome::Lost(e.to_string());
                    }
                    last_sent = Instant::now();
                }
            }
        }
    };

    // Frase rimasta aperta (fine audio o connessione persa): la chiudiamo.
    if let Some(kind) = proto.flush()
        && !events.send(kind).await
    {
        return Outcome::ConsumerGone;
    }
    outcome
}

/// Passa un messaggio al protocollo e inoltra gli eventi. `Ok(false)` se il
/// consumatore è sparito, `Err` se il protocollo chiede di chiudere.
async fn deliver<S: Protocol>(
    proto: &mut S,
    text: &str,
    origin: Option<Duration>,
    events: &EventSender,
) -> Result<bool, Outcome> {
    let kinds = match proto.on_text(text, origin.unwrap_or_default()) {
        Ok(kinds) => kinds,
        Err(e @ SttError::Connect(_)) => return Err(Outcome::Lost(e.to_string())),
        Err(e) => return Err(Outcome::Fatal(e.to_string())),
    };
    for kind in kinds {
        if !events.send(kind).await {
            return Ok(false);
        }
    }
    Ok(true)
}

/// f32 [-1, 1] → PCM s16 little-endian.
pub fn pcm16(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())
        .collect()
}

pub(crate) fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s.max(0.0))
}

/// Unisce due pezzi di testo; la punteggiatura si attacca senza spazio.
pub(crate) fn join(a: &str, b: &str) -> String {
    let b = b.trim();
    if a.is_empty() {
        b.to_owned()
    } else if b.is_empty() {
        a.to_owned()
    } else if b.starts_with(['.', ',', '!', '?', ';', ':', '…']) {
        format!("{a}{b}")
    } else {
        format!("{a} {b}")
    }
}

pub(crate) fn min_conf(a: Option<f32>, b: Option<f32>) -> Option<f32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Conteggio delle parole per lingua: la lingua di una frase è la prevalente.
#[derive(Debug, Default, Clone)]
pub(crate) struct LanguageTally(Vec<(String, usize)>);

impl LanguageTally {
    pub fn add(&mut self, language: &str, words: usize) {
        let language = base_code(language);
        if language.is_empty() || words == 0 {
            return;
        }
        match self.0.iter_mut().find(|(l, _)| *l == language) {
            Some((_, n)) => *n += words,
            None => self.0.push((language, words)),
        }
    }

    /// Lingua con più parole (a parità, la prima incontrata).
    pub fn top(&self) -> Option<String> {
        self.0
            .iter()
            .fold(None::<&(String, usize)>, |best, cur| match best {
                Some(b) if b.1 >= cur.1 => Some(b),
                _ => Some(cur),
            })
            .map(|(l, _)| l.clone())
    }
}

/// `it-IT` → `it`, minuscolo.
pub fn base_code(language: &str) -> String {
    language
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
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
    fn join_attaches_punctuation() {
        assert_eq!(join("", " ciao "), "ciao");
        assert_eq!(join("ciao", "a tutti"), "ciao a tutti");
        assert_eq!(join("ciao", "."), "ciao.");
        assert_eq!(join("ciao", ""), "ciao");
    }

    #[test]
    fn tally_picks_dominant_language() {
        let mut t = LanguageTally::default();
        assert_eq!(t.top(), None);
        t.add("it-IT", 2);
        t.add("en", 5);
        t.add("IT", 1);
        assert_eq!(t.top().as_deref(), Some("en"));
    }

    #[test]
    fn http_errors() {
        assert!(matches!(
            http_error(StatusCode::UNAUTHORIZED, ""),
            SttError::Unauthorized
        ));
        assert!(matches!(
            http_error(StatusCode::BAD_REQUEST, "x"),
            SttError::Protocol(_)
        ));
        assert!(matches!(
            http_error(StatusCode::TOO_MANY_REQUESTS, ""),
            SttError::Connect(_)
        ));
        assert!(matches!(
            http_error(StatusCode::BAD_GATEWAY, ""),
            SttError::Connect(_)
        ));
    }
}
