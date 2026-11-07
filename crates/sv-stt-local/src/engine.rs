//! `SttEngine` locale: VAD per canale + decodifica sul thread condiviso.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sv_audio::{AudioFrame, Channel};
use sv_stt::{EventSender, Language, Segment, SttEngine, SttEventKind, SttStream};
use sv_vad::{Segmenter, SpeechSegment, VadConfig, VadEvent};
use tokio::sync::mpsc;

use crate::lang;
use crate::recognizer::{Decoder, LocalModel};

/// Testo del parziale mostrato mentre il VAD sente parlato.
pub const SPEAKING_PLACEHOLDER: &str = "…";
/// Frasi in coda verso il decoder per canale.
const SEGMENT_QUEUE: usize = 32;
/// Oltre questo arretrato si scartano le frasi più vecchie: meglio perdere
/// una frase che accumulare ritardo per tutta la call.
const MAX_BACKLOG: usize = 4;

#[derive(Debug, Clone)]
pub struct LocalEngineConfig {
    pub model: LocalModel,
    pub vad: VadConfig,
    /// Thread di inferenza di onnxruntime.
    pub num_threads: i32,
}

/// Motore STT locale. Il modello si carica in background alla creazione:
/// gli stream emettono `Connected` quando è pronto, `Error` se fallisce.
pub struct LocalEngine {
    decoder: Arc<Decoder>,
    vad: VadConfig,
}

impl LocalEngine {
    pub fn new(config: LocalEngineConfig) -> Self {
        Self {
            decoder: Arc::new(Decoder::spawn(config.model, config.num_threads)),
            vad: config.vad,
        }
    }

    #[cfg(test)]
    fn with_decoder(decoder: Decoder, vad: VadConfig) -> Self {
        Self {
            decoder: Arc::new(decoder),
            vad,
        }
    }

    /// Attende il caricamento del modello (facoltativo: lo fanno gli stream).
    pub async fn ready(&self) -> Result<(), crate::LocalSttError> {
        self.decoder.wait_ready().await
    }
}

impl SttEngine for LocalEngine {
    fn name(&self) -> &'static str {
        "local"
    }

    fn start(&self, channel: Channel, language: Language) -> SttStream {
        let decoder = self.decoder.clone();
        let vad = self.vad.clone();
        SttStream::spawn(channel, move |audio, events| {
            run(audio, events, channel, language, decoder, vad)
        })
    }
}

async fn run(
    mut audio: mpsc::Receiver<AudioFrame>,
    events: EventSender,
    channel: Channel,
    language: Language,
    decoder: Arc<Decoder>,
    vad: VadConfig,
) {
    if let Err(e) = decoder.wait_ready().await {
        events.send(error(e.to_string())).await;
        return;
    }
    let mut segmenter = match Segmenter::new(&vad, channel) {
        Ok(s) => s,
        Err(e) => {
            events.send(error(e.to_string())).await;
            return;
        }
    };
    if !events.send(SttEventKind::Connected).await {
        return;
    }

    let speaking = Arc::new(AtomicBool::new(false));
    let (seg_tx, seg_rx) = mpsc::channel(SEGMENT_QUEUE);
    let decoding = tokio::spawn(decode_loop(
        seg_rx,
        events.clone(),
        language,
        decoder,
        speaking.clone(),
    ));

    let mut open = true;
    while open {
        let vad_events = match audio.recv().await {
            Some(frame) => segmenter.push(&frame),
            None => {
                open = false;
                segmenter.flush()
            }
        };
        for event in vad_events {
            match event {
                VadEvent::SpeechStarted { at, .. } => {
                    speaking.store(true, Ordering::Relaxed);
                    if !events.send(placeholder(at)).await {
                        return;
                    }
                }
                VadEvent::Segment(seg) => {
                    speaking.store(segmenter.is_speaking(), Ordering::Relaxed);
                    if seg_tx.try_send(seg).is_err() {
                        tracing::warn!(?channel, "coda decodifica locale piena: frase scartata");
                    }
                }
            }
        }
    }
    drop(seg_tx);
    let _ = decoding.await;
    events.send(SttEventKind::Disconnected).await;
}

async fn decode_loop(
    mut rx: mpsc::Receiver<SpeechSegment>,
    events: EventSender,
    language: Language,
    decoder: Arc<Decoder>,
    speaking: Arc<AtomicBool>,
) {
    let mut dropped = 0u64;
    while let Some(seg) = rx.recv().await {
        let seg = skip_backlog(seg, &mut rx, &mut dropped);
        let (start, end) = (seg.start, seg.end());
        let decoded = match decoder.decode(seg.samples).await {
            Ok(d) => d,
            Err(e) => {
                events.send(error(e.to_string())).await;
                return;
            }
        };
        tracing::debug!(
            channel = ?seg.channel,
            audio_ms = (end - start).as_millis() as u64,
            decode_ms = decoded.elapsed.as_millis() as u64,
            "frase decodificata"
        );
        let still_speaking = speaking.load(Ordering::Relaxed);
        let kind = if decoded.text.is_empty() {
            // Rumore scambiato per voce: si toglie il "…" se nessuno parla.
            if still_speaking {
                continue;
            }
            SttEventKind::Partial(Segment {
                start: end,
                end,
                ..Segment::default()
            })
        } else {
            let detected = match language {
                Language::Multi => lang::guess(&decoded.text).map(str::to_owned),
                fixed => Some(fixed.code().to_owned()),
            };
            SttEventKind::Final(Segment {
                id: 0,
                text: decoded.text,
                start,
                end,
                confidence: None,
                language: detected,
                speaker: None,
            })
        };
        let was_final = matches!(kind, SttEventKind::Final(_));
        if !events.send(kind).await {
            return;
        }
        // Il Final chiude il parziale: se intanto si parla, il "…" torna.
        if was_final && still_speaking && !events.send(placeholder(end)).await {
            return;
        }
    }
    if dropped > 0 {
        tracing::warn!(dropped, "frasi locali scartate per arretrato");
    }
}

/// Se la coda supera [`MAX_BACKLOG`], salta alle frasi più recenti.
fn skip_backlog(
    mut seg: SpeechSegment,
    rx: &mut mpsc::Receiver<SpeechSegment>,
    dropped: &mut u64,
) -> SpeechSegment {
    while rx.len() >= MAX_BACKLOG {
        match rx.try_recv() {
            Ok(next) => {
                *dropped += 1;
                tracing::warn!(channel = ?seg.channel, "decodifica in ritardo: frase scartata");
                seg = next;
            }
            Err(_) => break,
        }
    }
    seg
}

fn placeholder(at: Duration) -> SttEventKind {
    SttEventKind::Partial(Segment {
        text: SPEAKING_PLACEHOLDER.into(),
        start: at,
        end: at,
        ..Segment::default()
    })
}

fn error(message: String) -> SttEventKind {
    SttEventKind::Error { message }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::LocalSttError;
    use crate::recognizer::Transcribe;

    fn vad_model() -> Option<PathBuf> {
        let path = std::env::var_os("SILERO_VAD_MODEL").map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(
                "Library/Application Support/com.giuseppeconcas.sottovoce/models/silero-vad/silero_vad.onnx",
            )
        });
        if path.is_file() {
            Some(path)
        } else {
            eprintln!("SALTATO: modello Silero VAD assente ({})", path.display());
            None
        }
    }

    fn fixture() -> Vec<f32> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sv-vad/tests/fixtures/two_phrases_it.wav"
        );
        hound::WavReader::open(path)
            .unwrap()
            .samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / 32768.0)
            .collect()
    }

    /// Trascrittore finto: "frase N (durata ms)", oppure vuoto se `silent`.
    fn fake(silent: bool) -> Decoder {
        Decoder::spawn_with("fake", move || {
            let mut n = 0;
            Ok(Box::new(move |samples: &[f32]| {
                n += 1;
                if silent {
                    String::new()
                } else {
                    format!("Qual è la frase {n} ({} ms)", samples.len() / 16)
                }
            }) as Transcribe)
        })
    }

    async fn collect(engine: &LocalEngine, language: Language, audio: &[f32]) -> Vec<SttEventKind> {
        let mut stream = engine.start(Channel::Them, language);
        for (i, chunk) in audio.chunks(1024).enumerate() {
            stream
                .audio
                .send(AudioFrame {
                    channel: Channel::Them,
                    start: Duration::from_micros((i * 1024) as u64 * 1_000_000 / 16_000),
                    samples: Arc::from(chunk),
                })
                .await
                .unwrap();
        }
        drop(stream.audio);
        let mut out = Vec::new();
        while let Some(e) = stream.events.recv().await {
            assert_eq!(e.channel, Channel::Them);
            out.push(e.kind);
        }
        out
    }

    fn finals(events: &[SttEventKind]) -> Vec<&Segment> {
        events
            .iter()
            .filter_map(|e| match e {
                SttEventKind::Final(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn two_phrases_become_two_finals_with_placeholders() {
        let Some(model) = vad_model() else { return };
        let engine = LocalEngine::with_decoder(fake(false), VadConfig::new(model));
        let events = collect(&engine, Language::Multi, &fixture()).await;

        assert_eq!(events.first(), Some(&SttEventKind::Connected));
        assert_eq!(events.last(), Some(&SttEventKind::Disconnected));
        let finals = finals(&events);
        assert_eq!(finals.len(), 2, "{events:?}");
        assert!(finals[0].text.starts_with("Qual è la frase 1"));
        assert_eq!(finals[0].language.as_deref(), Some("it"));
        assert!(finals[0].start < finals[0].end && finals[0].end <= finals[1].start);
        assert!(
            (finals[1].start.as_secs_f64() - 5.0).abs() < 0.5,
            "{:?}",
            finals[1].start
        );
        let placeholders = events
            .iter()
            .filter(|e| matches!(e, SttEventKind::Partial(s) if s.text == SPEAKING_PLACEHOLDER))
            .count();
        assert_eq!(placeholders, 2);
    }

    #[tokio::test]
    async fn fixed_language_is_reported_and_empty_text_clears_placeholder() {
        let Some(model) = vad_model() else { return };
        let engine = LocalEngine::with_decoder(fake(false), VadConfig::new(model.clone()));
        let events = collect(&engine, Language::En, &fixture()).await;
        assert!(
            finals(&events)
                .iter()
                .all(|s| s.language.as_deref() == Some("en"))
        );

        let engine = LocalEngine::with_decoder(fake(true), VadConfig::new(model));
        let events = collect(&engine, Language::Multi, &fixture()).await;
        assert!(finals(&events).is_empty());
        let last_partial = events.iter().rev().find_map(|e| match e {
            SttEventKind::Partial(s) => Some(s),
            _ => None,
        });
        assert_eq!(last_partial.map(|s| s.text.as_str()), Some(""));
    }

    #[tokio::test]
    async fn load_failure_is_an_error_event() {
        let decoder = Decoder::spawn_with("broken", || {
            Err(LocalSttError::Load("modello rotto".into()))
        });
        let engine = LocalEngine::with_decoder(decoder, VadConfig::new("/nonexistent.onnx"));
        assert!(engine.ready().await.is_err());
        let events = collect(&engine, Language::Multi, &[0.0; 1024]).await;
        assert!(
            matches!(events.as_slice(), [SttEventKind::Error { message }] if message.contains("modello rotto")),
            "{events:?}"
        );
    }

    #[tokio::test]
    async fn backlog_keeps_newest_segments() {
        let (tx, mut rx) = mpsc::channel(SEGMENT_QUEUE);
        let seg = |s: u64| SpeechSegment {
            channel: Channel::Me,
            start: Duration::from_secs(s),
            samples: vec![0.0; 160],
        };
        for s in 1..=6 {
            tx.try_send(seg(s)).unwrap();
        }
        let mut dropped = 0;
        let first = rx.recv().await.unwrap();
        let picked = skip_backlog(first, &mut rx, &mut dropped);
        assert_eq!(picked.start, Duration::from_secs(3));
        assert_eq!(dropped, 2);
        assert_eq!(rx.len(), MAX_BACKLOG - 1);
    }
}
