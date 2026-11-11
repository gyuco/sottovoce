//! Diarization del canale "Loro" durante la sessione.
//!
//! Un task tiene l'audio recente di "Loro" (dal bus della sessione) e, per
//! ogni frase finale, ne ritaglia l'audio dai tempi della frase e lo passa al
//! [`Diarizer`] su un thread bloccante. Funziona con qualsiasi motore STT:
//! servono solo `start`/`end` della frase.
//!
//! Con i profili vocali attivi riconosce i parlanti già nominati in meeting
//! precedenti ([`VoiceHooks`]) e, a fine sessione, lascia la voce di ogni
//! parlante per aggiornare i profili.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use sv_audio::{AudioFrame, Channel};
use sv_diar::{
    Assignment, AudioRing, CampPlus, Diarizer, Embed, KnownVoice, MatchThresholds, SpeakerVoice,
    Verdict, VoiceMatch, VoiceStore,
};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

/// Audio di "Loro" conservato: copre il ritardo dei `Final` (Deepgram
/// endpointing, coda del motore locale) con ampio margine.
const AUDIO_WINDOW: Duration = Duration::from_secs(90);
/// Frasi in attesa di diarization.
const QUEUE: usize = 16;
/// Attesa massima per l'audio di fine frase non ancora arrivato sul bus.
const AUDIO_WAIT: Duration = Duration::from_millis(500);

pub struct DiarConfig {
    /// File ONNX di CAM++.
    pub model: PathBuf,
    /// Numero di altri partecipanti; 0 = automatico.
    pub speakers: u8,
    /// Profili vocali, solo se l'utente li ha attivati.
    pub voices: Option<VoiceProfiles>,
}

/// Profili vocali caricati all'avvio della sessione.
pub struct VoiceProfiles {
    pub store: VoiceStore,
    pub known: Vec<(KnownVoice, String)>,
}

/// Parlante riconosciuto in un profilo vocale, per la UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceEvent {
    pub speaker: u8,
    pub profile_id: String,
    pub name: String,
    pub similarity: f32,
    /// `assign` = nome da usare subito; `suggest` = da confermare.
    pub verdict: Verdict,
}

/// Profili vocali nella sessione.
pub struct VoiceHooks {
    /// Profili noti con il loro nome.
    pub known: Vec<(KnownVoice, String)>,
    pub on_match: Arc<dyn Fn(VoiceEvent) + Send + Sync>,
    /// Voce di ogni parlante, scritta a fine sessione.
    pub collected: Arc<Mutex<Vec<SpeakerVoice>>>,
}

/// Frase da attribuire; la risposta arriva su `reply`.
pub struct DiarJob {
    pub segment_id: u64,
    pub start: Duration,
    pub end: Duration,
    pub reply: oneshot::Sender<Assignment>,
}

/// Avvia il task. `bus` va sottoscritto prima dell'avvio della cattura.
/// Termina quando il chiamante chiude la coda dei job.
pub fn spawn(
    cfg: DiarConfig,
    bus: broadcast::Receiver<AudioFrame>,
    voices: Option<VoiceHooks>,
) -> (mpsc::Sender<DiarJob>, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(QUEUE);
    let task = tokio::spawn(async move {
        let DiarConfig {
            model,
            speakers,
            voices: _,
        } = cfg;
        let loaded = tokio::task::spawn_blocking(move || CampPlus::load(&model, 1)).await;
        match loaded {
            Ok(Ok(embedder)) => {
                let mut diarizer = Diarizer::new(embedder, speakers);
                if let Some(hooks) = &voices {
                    let known = hooks.known.iter().map(|(k, _)| k.clone()).collect();
                    diarizer = diarizer.with_voices(known, MatchThresholds::default());
                }
                run(diarizer, bus, rx, voices).await;
            }
            Ok(Err(e)) => tracing::warn!("riconoscimento dei parlanti disattivato: {e}"),
            Err(e) => tracing::warn!("riconoscimento dei parlanti disattivato: {e}"),
        }
        // Coda chiusa: il pump prosegue senza parlanti.
    });
    (tx, task)
}

async fn run<E: Embed + Send + 'static>(
    diarizer: Diarizer<E>,
    mut bus: broadcast::Receiver<AudioFrame>,
    mut jobs: mpsc::Receiver<DiarJob>,
    voices: Option<VoiceHooks>,
) {
    let names: HashMap<String, String> = voices
        .iter()
        .flat_map(|h| h.known.iter().map(|(k, n)| (k.id.clone(), n.clone())))
        .collect();
    let mut ring = AudioRing::new(AUDIO_WINDOW);
    let mut diarizer = Some(diarizer);
    let mut bus_open = true;
    let mut done = 0u64;
    loop {
        tokio::select! {
            biased;
            frame = bus.recv(), if bus_open => match frame {
                Ok(f) if f.channel == Channel::Them => ring.push(&f),
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(n, "diarization: frame audio persi");
                }
                Err(broadcast::error::RecvError::Closed) => bus_open = false,
            },
            job = jobs.recv() => {
                let Some(job) = job else { break };
                // L'audio di fine frase può essere ancora in coda sul bus.
                let deadline = tokio::time::Instant::now() + AUDIO_WAIT;
                while bus_open && ring.end_time() < job.end {
                    match tokio::time::timeout_at(deadline, bus.recv()).await {
                        Ok(Ok(f)) if f.channel == Channel::Them => ring.push(&f),
                        Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
                        Ok(Err(broadcast::error::RecvError::Closed)) => bus_open = false,
                        Err(_) => break,
                    }
                }
                let samples = ring.slice(job.start, job.end);
                let mut d = diarizer.take().expect("diarizer presente fuori dal calcolo");
                let id = job.segment_id;
                let Ok((back, result)) = tokio::task::spawn_blocking(move || {
                    let r = d.assign(id, &samples);
                    (d, r)
                })
                .await
                else {
                    tracing::warn!("diarization interrotta");
                    return;
                };
                diarizer = Some(back);
                tracing::debug!(segment = id, speaker = ?result.speaker, relabels = result.relabels.len(), "parlante");
                done += 1;
                if let Some(hooks) = &voices {
                    for m in &result.voices {
                        if let Some(event) = voice_event(m, &names) {
                            tracing::info!(speaker = m.speaker, similarity = m.similarity, verdict = ?m.verdict, "voce riconosciuta");
                            (hooks.on_match)(event);
                        }
                    }
                }
                let _ = job.reply.send(result);
            }
        }
    }
    if let (Some(hooks), Some(d)) = (&voices, &diarizer) {
        *hooks.collected.lock().unwrap_or_else(|e| e.into_inner()) = d.voices();
    }
    tracing::info!(segments = done, "diarization terminata");
}

fn voice_event(m: &VoiceMatch, names: &HashMap<String, String>) -> Option<VoiceEvent> {
    Some(VoiceEvent {
        speaker: m.speaker,
        profile_id: m.profile.clone(),
        name: names.get(&m.profile)?.clone(),
        similarity: m.similarity,
        verdict: m.verdict,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// Impronta = asse scelto dal valore medio dell'audio (1.0 → voce A,
    /// 2.0 → voce B).
    struct ByLevel;

    impl Embed for ByLevel {
        fn embed(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
            let mean = samples.iter().sum::<f32>() / samples.len().max(1) as f32;
            let axis = if mean > 1.5 { 1 } else { 0 };
            (!samples.is_empty()).then(|| {
                let mut v = vec![0.0; 4];
                v[axis] = 1.0;
                v
            })
        }
    }

    fn frame(channel: Channel, ms: u64, value: f32) -> AudioFrame {
        AudioFrame {
            channel,
            start: Duration::from_millis(ms),
            samples: Arc::from(vec![value; 1600]),
        }
    }

    async fn ask(tx: &mpsc::Sender<DiarJob>, id: u64, from_ms: u64, to_ms: u64) -> Assignment {
        let (reply, rx) = oneshot::channel();
        tx.send(DiarJob {
            segment_id: id,
            start: Duration::from_millis(from_ms),
            end: Duration::from_millis(to_ms),
            reply,
        })
        .await
        .unwrap();
        rx.await.unwrap()
    }

    #[tokio::test]
    async fn assigns_speakers_from_ring_audio() {
        let (bus, _) = broadcast::channel(256);
        let (tx, rx) = mpsc::channel(4);
        let task = tokio::spawn(run(Diarizer::new(ByLevel, 0), bus.subscribe(), rx, None));
        // 0-2.5 s voce A, 2.5-5 s voce B su "Loro"; "Io" ignorato. Un
        // parlante nuovo nasce solo da frasi ≥ 2 s.
        for i in 0..50u64 {
            let value = if i < 25 { 1.0 } else { 2.0 };
            bus.send(frame(Channel::Them, i * 100, value)).unwrap();
            bus.send(frame(Channel::Me, i * 100, 2.0)).unwrap();
        }
        assert_eq!(ask(&tx, 1, 0, 2400).await.speaker, Some(0));
        assert_eq!(ask(&tx, 2, 2500, 4900).await.speaker, Some(1));
        assert_eq!(ask(&tx, 3, 100, 1500).await.speaker, Some(0));
        drop(tx);
        drop(bus);
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn waits_briefly_for_late_audio() {
        let (bus, _) = broadcast::channel(256);
        let (tx, rx) = mpsc::channel(4);
        let rx_bus = bus.subscribe();
        tokio::spawn(run(Diarizer::new(ByLevel, 0), rx_bus, rx, None));
        let sender = bus.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            for i in 0..25u64 {
                sender.send(frame(Channel::Them, i * 100, 2.0)).unwrap();
            }
        });
        let got = ask(&tx, 1, 0, 2400).await;
        assert_eq!(got.speaker, Some(0));
    }
}
