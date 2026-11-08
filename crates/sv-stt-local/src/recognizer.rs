//! Riconoscitore offline (sherpa-onnx) e thread di decodifica condiviso.
//!
//! Il modello (centinaia di MB) è caricato una sola volta, su un thread
//! dedicato che decodifica una frase alla volta per tutti i canali: niente
//! inferenza sul runtime tokio e niente accessi concorrenti al modello.

use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use sherpa_onnx::{
    OfflineModelConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig,
};
use sv_audio::TARGET_SAMPLE_RATE;
use tokio::sync::{oneshot, watch};

use crate::LocalSttError;

/// Modello locale e cartella dei suoi file (vedi `sv-models`).
///
/// Whisper large-v3-turbo (int8, sherpa-onnx su CPU) è stato scartato: su
/// Apple Silicon decodifica a RTF ~2, cioè più lento del parlato.
#[derive(Debug, Clone)]
pub enum LocalModel {
    /// NVIDIA Parakeet TDT 0.6B v3 (transducer NeMo): 25 lingue europee,
    /// lingua riconosciuta in automatico.
    Parakeet { dir: PathBuf },
}

impl LocalModel {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Parakeet { .. } => "parakeet",
        }
    }
}

fn file(dir: &Path, name: &str) -> Result<Option<String>, LocalSttError> {
    let path = dir.join(name);
    if path.is_file() {
        Ok(Some(path.to_string_lossy().into_owned()))
    } else {
        Err(LocalSttError::MissingFile(path))
    }
}

/// Riconoscitore offline: una frase in, testo out.
pub struct Recognizer {
    inner: OfflineRecognizer,
}

impl Recognizer {
    /// Carica il modello su CPU (può richiedere qualche secondo).
    pub fn load(model: &LocalModel, num_threads: i32) -> Result<Self, LocalSttError> {
        Self::load_with_provider(model, num_threads, "cpu")
    }

    /// Come [`Recognizer::load`] con un execution provider di onnxruntime
    /// (`cpu`, `coreml`).
    pub fn load_with_provider(
        model: &LocalModel,
        num_threads: i32,
        provider: &str,
    ) -> Result<Self, LocalSttError> {
        let mut model_config = OfflineModelConfig {
            num_threads,
            provider: Some(provider.into()),
            ..Default::default()
        };
        match model {
            LocalModel::Parakeet { dir } => {
                model_config.transducer = OfflineTransducerModelConfig {
                    encoder: file(dir, "encoder.int8.onnx")?,
                    decoder: file(dir, "decoder.int8.onnx")?,
                    joiner: file(dir, "joiner.int8.onnx")?,
                };
                model_config.tokens = file(dir, "tokens.txt")?;
                model_config.model_type = Some("nemo_transducer".into());
            }
        }
        let config = OfflineRecognizerConfig {
            model_config,
            decoding_method: Some("greedy_search".into()),
            ..Default::default()
        };
        let inner = OfflineRecognizer::create(&config).ok_or_else(|| {
            LocalSttError::Load(format!("sherpa-onnx non ha caricato {}", model.label()))
        })?;
        Ok(Self { inner })
    }

    /// Trascrive una frase 16 kHz mono. Testo vuoto se non c'è parlato.
    pub fn transcribe(&self, samples: &[f32]) -> String {
        let stream = self.inner.create_stream();
        stream.accept_waveform(TARGET_SAMPLE_RATE as i32, samples);
        self.inner.decode(&stream);
        stream
            .get_result()
            .map(|r| r.text.trim().to_owned())
            .unwrap_or_default()
    }
}

/// Esito di una decodifica.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub text: String,
    /// Tempo di inferenza.
    pub elapsed: Duration,
}

#[derive(Debug, Clone)]
enum Ready {
    Loading,
    Ok,
    Failed(String),
}

/// Funzione di trascrizione posseduta dal thread di decodifica.
pub(crate) type Transcribe = Box<dyn FnMut(&[f32]) -> String + Send>;

struct Job {
    samples: Vec<f32>,
    reply: oneshot::Sender<Decoded>,
}

/// Lato async del thread di decodifica. Il thread termina quando l'ultimo
/// `Decoder` viene rilasciato.
pub(crate) struct Decoder {
    jobs: std_mpsc::Sender<Job>,
    ready: watch::Receiver<Ready>,
}

impl Decoder {
    /// Avvia il thread, che carica il modello e poi serve le richieste.
    pub(crate) fn spawn(model: LocalModel, num_threads: i32) -> Self {
        let label = model.label();
        Self::spawn_with(label, move || {
            let r = Recognizer::load(&model, num_threads)?;
            Ok(Box::new(move |samples: &[f32]| r.transcribe(samples)) as Transcribe)
        })
    }

    /// Come [`Decoder::spawn`] con un caricatore qualsiasi (test).
    pub(crate) fn spawn_with(
        label: &'static str,
        load: impl FnOnce() -> Result<Transcribe, LocalSttError> + Send + 'static,
    ) -> Self {
        let (jobs, rx) = std_mpsc::channel::<Job>();
        let (ready_tx, ready) = watch::channel(Ready::Loading);
        let spawned = std::thread::Builder::new()
            .name("stt-local".into())
            .spawn(move || {
                let started = Instant::now();
                let mut transcribe = match load() {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::error!("modello STT locale non caricato: {e}");
                        let _ = ready_tx.send(Ready::Failed(e.to_string()));
                        return;
                    }
                };
                tracing::info!(
                    model = label,
                    ms = started.elapsed().as_millis() as u64,
                    "modello STT locale caricato"
                );
                let _ = ready_tx.send(Ready::Ok);
                while let Ok(job) = rx.recv() {
                    let t = Instant::now();
                    let text = transcribe(&job.samples);
                    let _ = job.reply.send(Decoded {
                        text,
                        elapsed: t.elapsed(),
                    });
                }
            });
        if let Err(e) = spawned {
            tracing::error!("thread STT locale non avviato: {e}");
        }
        Self { jobs, ready }
    }

    /// Attende il caricamento del modello.
    pub(crate) async fn wait_ready(&self) -> Result<(), LocalSttError> {
        let mut ready = self.ready.clone();
        let state = ready
            .wait_for(|r| !matches!(r, Ready::Loading))
            .await
            .map(|r| r.clone())
            .map_err(|_| LocalSttError::Load("thread di decodifica terminato".into()))?;
        match state {
            Ready::Failed(e) => Err(LocalSttError::Load(e)),
            _ => Ok(()),
        }
    }

    /// Decodifica una frase sul thread dedicato.
    pub(crate) async fn decode(&self, samples: Vec<f32>) -> Result<Decoded, LocalSttError> {
        let (reply, rx) = oneshot::channel();
        self.jobs
            .send(Job { samples, reply })
            .map_err(|_| LocalSttError::Load("thread di decodifica terminato".into()))?;
        rx.await
            .map_err(|_| LocalSttError::Load("thread di decodifica terminato".into()))
    }
}
