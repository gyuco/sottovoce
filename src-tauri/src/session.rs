//! Sessione di trascrizione: cattura → STT (un stream per canale) → UI.
//!
//! Ogni sessione ha il proprio bus audio: fermare la cattura chiude il bus,
//! i forwarder terminano, i motori ricevono fine audio (CloseStream), emettono
//! gli ultimi `Final` e chiudono. Lo stop attende questa catena (con timeout).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use sv_audio::{AudioFrame, Capture, CaptureConfig, Channel, LevelEvent};
use sv_stt::{SpeakerUpdate, SttEngine, SttEvent, SttEventKind, SttStream};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::assist::{self, AssistConfig, AssistEvent, AssistLine};
use crate::diarize::{self, DiarConfig, DiarJob};
use crate::meeting::{MeetingContext, SharedContext};
use crate::settings::Settings;
use crate::translate::{self, Job, TranslationEvent, TranslatorConfig};

/// Capacità del bus audio: ~16 s di frame da ~64 ms per canale.
const FRAME_BUS_CAPACITY: usize = 512;
/// Tempo massimo per chiudere gli stream STT allo stop.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
/// Attesa massima del parlante prima di emettere comunque la frase.
const DIAR_TIMEOUT: Duration = Duration::from_secs(2);

/// Compiti LLM della sessione; `None` = disattivato (modello non scelto o
/// LLM non configurabile): la trascrizione funziona comunque.
#[derive(Default)]
pub struct LlmTasks {
    pub translation: Option<TranslatorConfig>,
    pub assist: Option<AssistConfig>,
}

/// Dove la sessione invia i propri eventi (canali IPC verso la UI).
pub struct Outputs {
    pub on_level: Box<dyn Fn(LevelEvent) + Send + Sync>,
    pub on_transcript: Arc<dyn Fn(SttEvent) + Send + Sync>,
    pub on_translation: Box<dyn Fn(TranslationEvent) + Send + Sync>,
    pub on_assist: Box<dyn Fn(AssistEvent) + Send + Sync>,
}

pub struct Session {
    capture: Capture,
    tasks: Vec<JoinHandle<()>>,
    context: SharedContext,
}

impl Session {
    /// `engine` già scelto dal chiamante (vedi `stt::resolve`).
    pub fn start(
        settings: &Settings,
        engine: Box<dyn SttEngine>,
        context: MeetingContext,
        llm: LlmTasks,
        diarization: Option<DiarConfig>,
        out: Outputs,
    ) -> Result<Self> {
        let Outputs {
            on_level,
            on_transcript,
            on_translation,
            on_assist,
        } = out;
        let system_audio = settings.system_audio && sv_audio::system::is_supported();
        let mut channels = vec![Channel::Me];
        if system_audio {
            channels.push(Channel::Them);
        }

        let context = SharedContext::new(context);
        let mut tasks = Vec::new();
        // ≈ origine dei timestamp dei frame (la cattura parte subito dopo).
        let started = Instant::now();
        let translating = llm.translation.is_some() && system_audio;
        // Solo "Loro" viene tradotto: senza audio di sistema niente traduttore.
        let translator = llm.translation.filter(|_| system_audio).map(|cfg| {
            let (tx, task) = translate::spawn(cfg, context.clone(), on_translation);
            tasks.push(task);
            tx
        });
        let assisting = llm.assist.is_some();
        let assistant = llm.assist.map(|cfg| {
            let (tx, task) = assist::spawn(cfg, context.clone(), started, on_assist);
            tasks.push(task);
            tx
        });

        let (frames, _) = broadcast::channel::<AudioFrame>(FRAME_BUS_CAPACITY);
        // Parlanti solo su "Loro" (senza audio di sistema non c'è nessuno).
        let diarizing = diarization.is_some() && system_audio;
        let diarizer = diarization.filter(|_| system_audio).map(|cfg| {
            let (tx, task) = diarize::spawn(cfg, frames.subscribe());
            tasks.push(task);
            tx
        });
        let ids = Arc::new(AtomicU64::new(1));
        for channel in channels {
            // Sottoscrizione prima dell'avvio cattura: nessun frame perso.
            let bus = frames.subscribe();
            let language = match channel {
                Channel::Me => settings.my_language,
                Channel::Them => settings.their_language,
            };
            let stream = engine.start(channel, language);
            let (translate, diarize) = match channel {
                Channel::Them => (translator.clone(), diarizer.clone()),
                Channel::Me => (None, None),
            };
            let sink = on_transcript.clone();
            tasks.extend(spawn_channel(
                stream,
                bus,
                started,
                ids.clone(),
                Followers {
                    translate,
                    assist: assistant.clone(),
                    diarize,
                },
                move |e| sink(e),
            ));
        }
        drop((translator, assistant, diarizer));

        let capture = Capture::start(
            CaptureConfig {
                mic_device_id: settings.mic_device_id.clone(),
                system_audio,
            },
            frames,
            on_level,
        );
        match capture {
            Ok(capture) => {
                tracing::info!(
                    provider = engine.name(),
                    local_only = settings.local_only,
                    me = settings.my_language.code(),
                    them = settings.their_language.code(),
                    system_audio,
                    with_context = context.prompt_block().is_some(),
                    translating,
                    assisting,
                    diarizing,
                    "sessione avviata"
                );
                Ok(Self {
                    capture,
                    tasks,
                    context,
                })
            }
            Err(e) => {
                // Il bus è già chiuso (sender droppato): i task terminano da soli,
                // ma non vogliamo connessioni appese.
                for t in &tasks {
                    t.abort();
                }
                Err(e).context("impossibile avviare la cattura")
            }
        }
    }

    /// Contesto del meeting, letto dai prompt a ogni chiamata.
    pub fn context(&self) -> &SharedContext {
        &self.context
    }

    pub async fn stop(self) {
        let Self { capture, tasks, .. } = self;
        // Join dei thread di cattura (pochi ms): chiude il bus audio.
        let _ = tokio::task::spawn_blocking(move || capture.stop()).await;
        let aborts: Vec<_> = tasks.iter().map(JoinHandle::abort_handle).collect();
        let all = async {
            for t in tasks {
                let _ = t.await;
            }
        };
        if tokio::time::timeout(STOP_TIMEOUT, all).await.is_err() {
            tracing::warn!("stream STT non chiusi entro {STOP_TIMEOUT:?}: interrotti");
            for a in aborts {
                a.abort();
            }
        }
        tracing::info!("sessione fermata");
    }
}

/// Task che ricevono le frasi finali di un canale.
#[derive(Default)]
struct Followers {
    translate: Option<mpsc::Sender<Job>>,
    assist: Option<mpsc::Sender<AssistLine>>,
    /// Diarization ("Loro"): il `Final` aspetta il parlante.
    diarize: Option<mpsc::Sender<DiarJob>>,
}

/// Due task per canale: bus audio → motore, eventi motore → UI (e frasi
/// finali → traduttore e live assist, se attivi). `ids` numera i `Final`
/// della sessione, su tutti i canali.
fn spawn_channel(
    stream: SttStream,
    mut bus: broadcast::Receiver<AudioFrame>,
    started: Instant,
    ids: Arc<AtomicU64>,
    followers: Followers,
    on_transcript: impl Fn(SttEvent) + Send + 'static,
) -> [JoinHandle<()>; 2] {
    let Followers {
        translate,
        assist,
        diarize,
    } = followers;
    let SttStream {
        channel,
        audio,
        mut events,
        task: engine,
    } = stream;

    let forward = tokio::spawn(async move {
        let mut dropped = 0u64;
        loop {
            match bus.recv().await {
                Ok(f) if f.channel == channel => match audio.try_send(f) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        dropped += 1;
                        if dropped.is_power_of_two() {
                            tracing::warn!(?channel, dropped, "coda STT piena: frame scartati");
                        }
                    }
                    // Motore terminato (errore non recuperabile).
                    Err(TrySendError::Closed(_)) => break,
                },
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(?channel, n, "bus audio in ritardo: frame persi");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        // Drop di `audio` = fine audio per il motore.
    });

    let pump = tokio::spawn(async move {
        let mut stats = LatencyStats::default();
        let mut dropped = 0u64;
        while let Some(mut event) = events.recv().await {
            stats.observe(&event, started);
            let mut relabels = Vec::new();
            let job = match &mut event.kind {
                SttEventKind::Final(seg) => {
                    seg.id = ids.fetch_add(1, Ordering::Relaxed);
                    if let Some(tx) = &diarize
                        && let Some(found) = speaker_of(tx, seg).await
                    {
                        seg.speaker = found.speaker;
                        relabels = found.relabels;
                    }
                    if let Some(tx) = &assist
                        && let Err(TrySendError::Full(_)) = tx.try_send(AssistLine {
                            channel,
                            text: seg.text.clone(),
                            speaker: seg.speaker,
                        })
                    {
                        tracing::warn!(?channel, "coda live assist piena: frase scartata");
                    }
                    translate.as_ref().map(|_| Job {
                        segment_id: seg.id,
                        text: seg.text.clone(),
                        language: seg.language.clone(),
                        speaker: seg.speaker,
                    })
                }
                _ => None,
            };
            on_transcript(event);
            if !relabels.is_empty() {
                on_transcript(SttEvent {
                    channel,
                    kind: SttEventKind::Speakers {
                        updates: relabels
                            .into_iter()
                            .map(|(segment_id, speaker)| SpeakerUpdate {
                                segment_id,
                                speaker,
                            })
                            .collect(),
                    },
                });
            }
            if let (Some(tx), Some(job)) = (&translate, job)
                && let Err(TrySendError::Full(_)) = tx.try_send(job)
            {
                dropped += 1;
                tracing::warn!(
                    ?channel,
                    dropped,
                    "coda traduzione piena: frase non tradotta"
                );
            }
        }
        drop(diarize);
        // Drop dei sender: traduttore e assist chiudono (dopo le richieste
        // in corso) quando tutti i canali hanno finito.
        drop((translate, assist));
        stats.report(channel);
        let _ = engine.await;
    });

    [forward, pump]
}

/// Parlante della frase dal task di diarization; `None` se il task non c'è
/// più (modello non caricato), è in ritardo o la coda è piena.
async fn speaker_of(
    tx: &mpsc::Sender<DiarJob>,
    seg: &sv_stt::Segment,
) -> Option<sv_diar::Assignment> {
    let (reply, rx) = oneshot::channel();
    let job = DiarJob {
        segment_id: seg.id,
        start: seg.start,
        end: seg.end,
        reply,
    };
    if let Err(TrySendError::Full(_)) = tx.try_send(job) {
        tracing::warn!("coda diarization piena: frase senza parlante");
        return None;
    }
    match tokio::time::timeout(DIAR_TIMEOUT, rx).await {
        Ok(Ok(found)) => Some(found),
        Ok(Err(_)) => None,
        Err(_) => {
            tracing::warn!(segment = seg.id, "parlante non pronto in tempo");
            None
        }
    }
}

/// Ritardo tra la fine dell'audio di un segmento e l'arrivo dell'evento.
/// Riepilogo (p50/p95/max) a fine sessione per canale.
#[derive(Default)]
struct LatencyStats {
    first_partial_ms: Option<u64>,
    partial_ms: Vec<u64>,
    final_ms: Vec<u64>,
}

impl LatencyStats {
    fn observe(&mut self, event: &SttEvent, started: Instant) {
        let (seg, is_final) = match &event.kind {
            SttEventKind::Partial(s) => (s, false),
            SttEventKind::Final(s) => (s, true),
            _ => return,
        };
        let lag = started.elapsed().saturating_sub(seg.end).as_millis() as u64;
        if is_final {
            tracing::debug!(channel = ?event.channel, lag_ms = lag, "final STT");
            self.final_ms.push(lag);
        } else {
            if self.first_partial_ms.is_none() {
                tracing::info!(channel = ?event.channel, lag_ms = lag, "primo parziale STT");
                self.first_partial_ms = Some(lag);
            }
            self.partial_ms.push(lag);
        }
    }

    fn report(mut self, channel: Channel) {
        let [p50, p95, max] = percentiles(&mut self.partial_ms);
        let [f50, f95, fmax] = percentiles(&mut self.final_ms);
        tracing::info!(
            ?channel,
            first_partial_ms = self.first_partial_ms,
            partials = self.partial_ms.len(),
            partial_p50_ms = p50,
            partial_p95_ms = p95,
            partial_max_ms = max,
            finals = self.final_ms.len(),
            final_p50_ms = f50,
            final_p95_ms = f95,
            final_max_ms = fmax,
            "riepilogo latenza STT"
        );
    }
}

/// [p50, p95, max] (0 se vuoto).
pub(crate) fn percentiles(v: &mut [u64]) -> [u64; 3] {
    if v.is_empty() {
        return [0; 3];
    }
    v.sort_unstable();
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    [at(0.5), at(0.95), v[v.len() - 1]]
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    #[test]
    fn percentiles_basic() {
        assert_eq!(super::percentiles(&mut []), [0, 0, 0]);
        let mut v: Vec<u64> = (1..=100).rev().collect();
        assert_eq!(super::percentiles(&mut v), [51, 95, 100]);
    }

    use sv_stt::fake::FakeEngine;
    use sv_stt::{Language, SttEventKind};

    use super::*;

    fn frame(channel: Channel, i: u64) -> AudioFrame {
        AudioFrame {
            channel,
            start: Duration::from_millis(i * 100),
            samples: Arc::from(vec![0.0f32; 1600]),
        }
    }

    #[tokio::test]
    async fn forwards_only_own_channel_and_closes_with_bus() {
        let (bus, _) = broadcast::channel(64);
        let engine = FakeEngine {
            frames_per_final: 2,
        };
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        let tasks = spawn_channel(
            engine.start(Channel::Them, Language::Multi),
            bus.subscribe(),
            Instant::now(),
            Arc::new(AtomicU64::new(1)),
            Followers::default(),
            move |e| sink.lock().unwrap().push(e),
        );
        for i in 0..3 {
            bus.send(frame(Channel::Me, i)).unwrap();
            bus.send(frame(Channel::Them, i)).unwrap();
        }
        drop(bus);
        for t in tasks {
            tokio::time::timeout(Duration::from_secs(2), t)
                .await
                .expect("i task devono chiudersi con il bus")
                .unwrap();
        }
        let got = got.lock().unwrap();
        assert!(got.iter().all(|e| e.channel == Channel::Them));
        let finals: Vec<_> = got
            .iter()
            .filter_map(|e| match &e.kind {
                SttEventKind::Final(s) => Some(s.text.as_str()),
                _ => None,
            })
            .collect();
        // 3 frame "Loro": una frase da 2 + quella chiusa a fine audio.
        assert_eq!(finals, ["w1 w2", "w1"]);
        assert_eq!(got.last().unwrap().kind, SttEventKind::Disconnected);
    }

    #[tokio::test]
    async fn finals_get_ids_and_go_to_translator() {
        let (bus, _) = broadcast::channel(64);
        let engine = FakeEngine {
            frames_per_final: 2,
        };
        let (tx, mut jobs) = mpsc::channel(8);
        let finals = Arc::new(Mutex::new(Vec::new()));
        let sink = finals.clone();
        let tasks = spawn_channel(
            engine.start(Channel::Them, Language::Multi),
            bus.subscribe(),
            Instant::now(),
            Arc::new(AtomicU64::new(5)),
            Followers {
                translate: Some(tx),
                assist: None,
                diarize: None,
            },
            move |e| {
                if let SttEventKind::Final(s) = e.kind {
                    sink.lock().unwrap().push(s.id);
                }
            },
        );
        for i in 0..4 {
            bus.send(frame(Channel::Them, i)).unwrap();
        }
        drop(bus);
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(*finals.lock().unwrap(), [5, 6]);
        let mut sent = Vec::new();
        while let Some(job) = jobs.recv().await {
            sent.push((job.segment_id, job.text));
        }
        // Il canale si chiude con il pump: il traduttore termina.
        assert_eq!(sent, [(5, "w1 w2".to_owned()), (6, "w1 w2".to_owned())]);
    }
}
