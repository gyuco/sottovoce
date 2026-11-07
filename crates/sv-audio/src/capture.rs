use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ringbuf::traits::Consumer;
use tokio::sync::broadcast;

use crate::error::AudioError;
use crate::frame::{AudioFrame, Channel, LevelEvent, TARGET_SAMPLE_RATE};
use crate::level::LevelMeter;
use crate::resample::MonoResampler;
use crate::source::OpenedSource;
use crate::{mic, system};

/// Finestra del meter di livello.
const LEVEL_WINDOW: Duration = Duration::from_millis(50);
/// Intervallo di polling del worker sul ring buffer.
const POLL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Default)]
pub struct CaptureConfig {
    /// ID del microfono (da [`crate::list_input_devices`]); `None` = default di sistema.
    pub mic_device_id: Option<String>,
    /// Cattura anche l'audio di sistema ("Loro").
    pub system_audio: bool,
}

type LevelSink = Arc<dyn Fn(LevelEvent) + Send + Sync>;

/// Sessione di cattura attiva. Il drop ferma tutti i canali.
pub struct Capture {
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}

impl Capture {
    /// Avvia i canali richiesti. Ritorna errore se una sorgente non si apre.
    ///
    /// `frames` riceve l'audio 16 kHz mono di entrambi i canali; `on_level`
    /// riceve i livelli a ~20 Hz per canale.
    pub fn start(
        config: CaptureConfig,
        frames: broadcast::Sender<AudioFrame>,
        on_level: impl Fn(LevelEvent) + Send + Sync + 'static,
    ) -> Result<Self, AudioError> {
        let stop = Arc::new(AtomicBool::new(false));
        let on_level: LevelSink = Arc::new(on_level);
        let started = Instant::now();
        let mut capture = Self {
            stop: stop.clone(),
            workers: Vec::new(),
        };

        let mic_id = config.mic_device_id.clone();
        capture.workers.push(spawn_worker(
            Channel::Me,
            move || mic::open(mic_id.as_deref()),
            stop.clone(),
            started,
            frames.clone(),
            on_level.clone(),
        )?);

        if config.system_audio {
            capture.workers.push(spawn_worker(
                Channel::Them,
                system::open,
                stop.clone(),
                started,
                frames,
                on_level,
            )?);
        }
        Ok(capture)
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Avvia un thread che apre la sorgente (alcune risorse, es. lo stream cpal
/// su macOS, non sono `Send`: devono nascere e morire nello stesso thread),
/// poi consuma il ring buffer finché `stop` non diventa true.
fn spawn_worker(
    channel: Channel,
    open: impl FnOnce() -> Result<OpenedSource, AudioError> + Send + 'static,
    stop: Arc<AtomicBool>,
    started: Instant,
    frames: broadcast::Sender<AudioFrame>,
    on_level: LevelSink,
) -> Result<JoinHandle<()>, AudioError> {
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), AudioError>>();
    let handle = std::thread::Builder::new()
        .name(format!("sv-audio-{channel:?}").to_lowercase())
        .spawn(move || {
            let src = match open() {
                Ok(src) => src,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let mut pipeline = match Pipeline::new(channel, &src, started) {
                Ok(p) => p,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));
            tracing::info!(
                ?channel,
                rate = src.sample_rate,
                channels = src.channels,
                "cattura avviata"
            );
            run(src, &mut pipeline, &stop, &frames, on_level.as_ref());
            tracing::info!(?channel, "cattura fermata");
        })
        .map_err(|_| AudioError::WorkerDied)?;

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(e)
        }
        Err(_) => Err(AudioError::WorkerDied),
    }
}

struct Pipeline {
    channel: Channel,
    channels: usize,
    meter: LevelMeter,
    resampler: MonoResampler,
    /// Campioni a 16 kHz pubblicati finora: base per il timestamp dei frame.
    emitted: u64,
    offset: Duration,
    raw: Vec<f32>,
    mono: Vec<f32>,
}

impl Pipeline {
    fn new(channel: Channel, src: &OpenedSource, started: Instant) -> Result<Self, AudioError> {
        let window = (src.sample_rate as f64 * LEVEL_WINDOW.as_secs_f64()) as usize;
        Ok(Self {
            channel,
            channels: src.channels.max(1),
            meter: LevelMeter::new(window),
            resampler: MonoResampler::new(src.sample_rate, TARGET_SAMPLE_RATE)?,
            emitted: 0,
            offset: started.elapsed(),
            raw: vec![0.0; 8192 * src.channels.max(1)],
            mono: Vec::with_capacity(8192),
        })
    }
}

fn run(
    mut src: OpenedSource,
    p: &mut Pipeline,
    stop: &AtomicBool,
    frames: &broadcast::Sender<AudioFrame>,
    on_level: &(dyn Fn(LevelEvent) + Send + Sync),
) {
    while !stop.load(Ordering::Relaxed) {
        let n = src.consumer.pop_slice(&mut p.raw);
        if n == 0 {
            std::thread::sleep(POLL);
            continue;
        }
        // Downmix a mono (scarta un eventuale frame incompleto in coda).
        let ch = p.channels;
        p.mono.clear();
        p.mono.extend(
            p.raw[..n - n % ch]
                .chunks_exact(ch)
                .map(|f| f.iter().sum::<f32>() / ch as f32),
        );

        let channel = p.channel;
        p.meter.push(&p.mono, |rms_db, peak_db| {
            on_level(LevelEvent {
                channel,
                rms_db,
                peak_db,
            });
        });

        let (emitted, offset) = (&mut p.emitted, p.offset);
        p.resampler.push(&p.mono, |block| {
            let start =
                offset + Duration::from_secs_f64(*emitted as f64 / f64::from(TARGET_SAMPLE_RATE));
            *emitted += block.len() as u64;
            // Errore = nessun subscriber: normale finché lo STT non è collegato.
            let _ = frames.send(AudioFrame {
                channel,
                start,
                samples: Arc::from(block),
            });
        });
    }
    drop(src);
}
