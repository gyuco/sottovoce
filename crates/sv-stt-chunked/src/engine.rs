//! `SttEngine` a blocchi: VAD locale per canale, una richiesta HTTP per
//! blocco di parlato, risultato come `Final`. Niente parziali dal servizio:
//! mentre si parla il canale mostra "…" come il motore locale.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sv_audio::{AudioFrame, Channel, TARGET_SAMPLE_RATE};
use sv_stt::{EventSender, Language, Segment, SttEngine, SttEventKind, SttStream};
use sv_stt_local::{SPEAKING_PLACEHOLDER, lang};
use sv_vad::{Segmenter, SpeechSegment, VadConfig, VadEvent};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::{ChunkError, Transcriber, Transcript};

/// Frasi in coda verso le richieste per canale.
const SEGMENT_QUEUE: usize = 32;
/// Oltre questo arretrato si salta alle frasi più recenti.
const MAX_BACKLOG: usize = 4;
/// Tentativi per un blocco su errori temporanei (rete, 5xx, 429).
const MAX_ATTEMPTS: u32 = 3;
/// Blocchi falliti di fila prima di arrendersi con un errore.
const MAX_CONSECUTIVE_FAILURES: u32 = 5;
/// Attesa massima suggerita da un 429 che rispettiamo.
const MAX_RATE_WAIT: Duration = Duration::from_secs(10);
/// Silenzio inserito tra frasi unite nello stesso blocco.
const JOIN_GAP: Duration = Duration::from_millis(200);

/// Quanto audio mettere in una richiesta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Batching {
    /// Si uniscono frasi finché il blocco è più corto di così (0 = una frase
    /// per richiesta). Serve ai servizi che fatturano un minimo per richiesta
    /// o limitano le richieste al minuto.
    pub min_audio: Duration,
    /// Attesa massima dalla prima frase del blocco prima di inviarlo.
    pub max_wait: Duration,
    /// Durata massima di un blocco (limite del servizio).
    pub max_audio: Duration,
}

impl Batching {
    pub const PER_SENTENCE: Self = Self {
        min_audio: Duration::ZERO,
        max_wait: Duration::ZERO,
        max_audio: Duration::from_secs(55),
    };
}

pub struct ChunkedEngine<T> {
    transcriber: Arc<T>,
    vad: VadConfig,
}

impl<T: Transcriber> ChunkedEngine<T> {
    pub fn new(transcriber: T, vad: VadConfig) -> Self {
        Self {
            transcriber: Arc::new(transcriber),
            vad,
        }
    }
}

impl<T: Transcriber> SttEngine for ChunkedEngine<T> {
    fn name(&self) -> &'static str {
        self.transcriber.name()
    }

    fn start(&self, channel: Channel, language: Language) -> SttStream {
        let transcriber = self.transcriber.clone();
        let vad = self.vad.clone();
        SttStream::spawn(channel, move |audio, events| {
            run(audio, events, channel, language, transcriber, vad)
        })
    }
}

async fn run<T: Transcriber>(
    mut audio: mpsc::Receiver<AudioFrame>,
    events: EventSender,
    channel: Channel,
    language: Language,
    transcriber: Arc<T>,
    vad: VadConfig,
) {
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
    let worker = tokio::spawn(transcribe_loop(
        seg_rx,
        events.clone(),
        language,
        transcriber,
        speaking.clone(),
    ));

    let mut open = true;
    while open && !worker.is_finished() {
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
                        tracing::warn!(?channel, "coda trascrizione piena: frase scartata");
                    }
                }
            }
        }
    }
    drop(seg_tx);
    // `false` = errore definitivo già segnalato: niente `Disconnected`.
    if worker.await.unwrap_or(false) {
        events.send(SttEventKind::Disconnected).await;
    }
}

/// Blocco di audio da trascrivere.
#[derive(Debug)]
pub(crate) struct Chunk {
    pub start: Duration,
    pub end: Duration,
    pub samples: Vec<f32>,
}

/// Raccoglie le frasi dalla coda in blocchi secondo `batching`.
pub(crate) struct Batcher {
    rx: mpsc::Receiver<SpeechSegment>,
    batching: Batching,
    /// Frase che non stava nel blocco precedente: apre il prossimo.
    carry: Option<SpeechSegment>,
    pub dropped: u64,
}

impl Batcher {
    pub fn new(rx: mpsc::Receiver<SpeechSegment>, batching: Batching) -> Self {
        Self {
            rx,
            batching,
            carry: None,
            dropped: 0,
        }
    }

    /// Prossimo blocco: una frase, o più frasi unite. `None` quando la coda
    /// è chiusa e vuota.
    pub async fn next(&mut self) -> Option<Chunk> {
        let first = match self.carry.take() {
            Some(seg) => seg,
            None => skip_backlog(self.rx.recv().await?, &mut self.rx, &mut self.dropped),
        };
        let deadline = Instant::now() + self.batching.max_wait;
        let mut chunk = Chunk {
            start: first.start,
            end: first.end(),
            samples: first.samples,
        };
        let gap = vec![0.0; JOIN_GAP.as_millis() as usize * TARGET_SAMPLE_RATE as usize / 1000];
        while duration(chunk.samples.len()) < self.batching.min_audio {
            let next = match tokio::time::timeout_at(deadline, self.rx.recv()).await {
                Ok(Some(seg)) => seg,
                Ok(None) | Err(_) => break,
            };
            let total = chunk.samples.len() + gap.len() + next.samples.len();
            if duration(total) > self.batching.max_audio {
                self.carry = Some(next);
                break;
            }
            chunk.samples.extend_from_slice(&gap);
            chunk.samples.extend_from_slice(&next.samples);
            chunk.end = next.end();
        }
        Some(chunk)
    }
}

fn duration(samples: usize) -> Duration {
    Duration::from_secs_f64(samples as f64 / f64::from(TARGET_SAMPLE_RATE))
}

/// `true` = fine normale, `false` = errore definitivo (già inviato).
async fn transcribe_loop<T: Transcriber>(
    rx: mpsc::Receiver<SpeechSegment>,
    events: EventSender,
    language: Language,
    transcriber: Arc<T>,
    speaking: Arc<AtomicBool>,
) -> bool {
    let mut batcher = Batcher::new(rx, transcriber.batching());
    let mut failures = 0u32;
    while let Some(chunk) = batcher.next().await {
        let (start, end) = (chunk.start, chunk.end);
        let result = transcribe_with_retry(&*transcriber, chunk.samples, language).await;
        let still_speaking = speaking.load(Ordering::Relaxed);
        let transcript = match result {
            Ok(t) => {
                failures = 0;
                t
            }
            Err(ChunkError::Fatal(message)) => {
                events.send(error(message)).await;
                return false;
            }
            Err(ChunkError::Unauthorized) => {
                events
                    .send(error(
                        "API key non valida o non autorizzata per il servizio di trascrizione"
                            .into(),
                    ))
                    .await;
                return false;
            }
            Err(e) => {
                failures += 1;
                tracing::warn!(
                    failures,
                    "{}: blocco non trascritto: {e}",
                    transcriber.name()
                );
                if failures >= MAX_CONSECUTIVE_FAILURES {
                    events
                        .send(error(format!(
                            "servizio di trascrizione non raggiungibile: {e}"
                        )))
                        .await;
                    return false;
                }
                Transcript::default()
            }
        };
        let kind = if transcript.text.trim().is_empty() {
            if still_speaking {
                continue;
            }
            // Niente testo e nessuno parla: si toglie il "…".
            SttEventKind::Partial(Segment {
                start: end,
                end,
                ..Segment::default()
            })
        } else {
            let text = transcript.text.trim().to_owned();
            let detected = match language {
                Language::Multi => transcript
                    .language
                    .or_else(|| lang::guess(&text).map(str::to_owned)),
                fixed => Some(fixed.code().to_owned()),
            };
            SttEventKind::Final(Segment {
                text,
                start,
                end,
                confidence: transcript.confidence,
                language: detected,
                ..Segment::default()
            })
        };
        let was_final = matches!(kind, SttEventKind::Final(_));
        if !events.send(kind).await {
            return true;
        }
        if was_final && still_speaking && !events.send(placeholder(end)).await {
            return true;
        }
    }
    if batcher.dropped > 0 {
        tracing::warn!(dropped = batcher.dropped, "frasi scartate per arretrato");
    }
    true
}

async fn transcribe_with_retry<T: Transcriber>(
    transcriber: &T,
    samples: Vec<f32>,
    language: Language,
) -> Result<Transcript, ChunkError> {
    let samples: Arc<[f32]> = samples.into();
    let mut attempt = 0;
    loop {
        attempt += 1;
        match transcriber.transcribe(samples.clone(), language).await {
            Err(ChunkError::RateLimited(wait)) if attempt < MAX_ATTEMPTS => {
                let wait = wait.unwrap_or(Duration::from_secs(2)).min(MAX_RATE_WAIT);
                tokio::time::sleep(wait).await;
            }
            Err(ChunkError::Temporary(_)) if attempt < MAX_ATTEMPTS => {
                tokio::time::sleep(Duration::from_millis(500 << attempt)).await;
            }
            other => return other,
        }
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
                tracing::warn!(channel = ?seg.channel, "trascrizione in ritardo: frase scartata");
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
    use super::*;

    fn seg(start_ms: u64, len_ms: u64) -> SpeechSegment {
        SpeechSegment {
            channel: Channel::Them,
            start: Duration::from_millis(start_ms),
            samples: vec![0.1; (len_ms * 16) as usize],
        }
    }

    #[tokio::test]
    async fn per_sentence_sends_each_segment() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(seg(0, 1000)).await.unwrap();
        tx.send(seg(2000, 500)).await.unwrap();
        drop(tx);
        let mut b = Batcher::new(rx, Batching::PER_SENTENCE);
        let a = b.next().await.unwrap();
        assert_eq!((a.start, a.end), (Duration::ZERO, Duration::from_secs(1)));
        assert_eq!(b.next().await.unwrap().start, Duration::from_secs(2));
        assert!(b.next().await.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn batching_merges_until_min_or_deadline() {
        let batching = Batching {
            min_audio: Duration::from_secs(3),
            max_wait: Duration::from_secs(5),
            max_audio: Duration::from_secs(55),
        };
        let (tx, rx) = mpsc::channel(8);
        tx.send(seg(0, 2000)).await.unwrap();
        tx.send(seg(3000, 2000)).await.unwrap();
        tx.send(seg(6000, 1000)).await.unwrap();
        let mut b = Batcher::new(rx, batching);
        let a = b.next().await.unwrap();
        // 2 s + 0,2 s di pausa + 2 s ≥ 3 s: due frasi unite.
        assert_eq!((a.start, a.end), (Duration::ZERO, Duration::from_secs(5)));
        assert_eq!(a.samples.len(), 4200 * 16);
        // Terza frase da sola: scade l'attesa (tempo simulato).
        assert_eq!(b.next().await.unwrap().start, Duration::from_secs(6));
        drop(tx);
        assert!(b.next().await.is_none());
    }

    #[tokio::test]
    async fn overflow_opens_next_chunk() {
        let batching = Batching {
            min_audio: Duration::from_secs(10),
            max_wait: Duration::from_secs(5),
            max_audio: Duration::from_secs(4),
        };
        let (tx, rx) = mpsc::channel(8);
        tx.send(seg(0, 3000)).await.unwrap();
        tx.send(seg(4000, 3000)).await.unwrap();
        drop(tx);
        let mut b = Batcher::new(rx, batching);
        assert_eq!(b.next().await.unwrap().end, Duration::from_secs(3));
        assert_eq!(b.next().await.unwrap().start, Duration::from_secs(4));
        assert!(b.next().await.is_none());
    }
}
