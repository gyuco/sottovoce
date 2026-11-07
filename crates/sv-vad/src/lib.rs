//! Rilevamento del parlato per Sottovoce.
//!
//! Un [`Segmenter`] per canale riceve gli [`AudioFrame`] (16 kHz mono) e li
//! taglia in frasi ([`SpeechSegment`]) con Silero VAD (via `sherpa-onnx`):
//! una frase si chiude dopo un silenzio minimo o alla durata massima, con un
//! po' di audio prima e dopo (padding) per non tagliare le parole ai bordi.
//! I motori STT locali, che non fanno vero streaming, trascrivono una frase
//! alla volta.

mod history;

use std::path::PathBuf;
use std::time::Duration;

use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use sv_audio::{AudioFrame, Channel, TARGET_SAMPLE_RATE};

use history::History;

const RATE: u64 = TARGET_SAMPLE_RATE as u64;
/// Finestra di Silero VAD a 16 kHz (campioni).
const WINDOW: i32 = 512;
/// Margine del buffer di storia oltre la frase più lunga.
const HISTORY_SLACK: Duration = Duration::from_secs(3);

/// Parametri del VAD.
#[derive(Debug, Clone)]
pub struct VadConfig {
    /// File `silero_vad.onnx`.
    pub model: PathBuf,
    /// Probabilità di parlato sopra cui una finestra è "voce" (0–1).
    pub threshold: f32,
    /// Silenzio che chiude una frase.
    pub min_silence: Duration,
    /// Parlato più breve di così viene ignorato (colpi, click).
    pub min_speech: Duration,
    /// Oltre questa durata la frase viene tagliata comunque.
    pub max_speech: Duration,
    /// Audio aggiunto prima e dopo ogni frase.
    pub padding: Duration,
}

impl VadConfig {
    pub fn new(model: impl Into<PathBuf>) -> Self {
        Self {
            model: model.into(),
            threshold: 0.5,
            min_silence: Duration::from_millis(500),
            min_speech: Duration::from_millis(250),
            max_speech: Duration::from_secs(15),
            padding: Duration::from_millis(200),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VadError {
    #[error("modello VAD non trovato: {0}")]
    ModelNotFound(PathBuf),
    #[error("impossibile inizializzare Silero VAD ({0})")]
    Init(PathBuf),
}

/// Una frase di parlato di un canale.
#[derive(Debug, Clone)]
pub struct SpeechSegment {
    pub channel: Channel,
    /// Tempo dall'avvio della cattura al primo campione (padding incluso).
    pub start: Duration,
    /// Audio 16 kHz mono, padding incluso.
    pub samples: Vec<f32>,
}

impl SpeechSegment {
    pub fn duration(&self) -> Duration {
        samples_to_duration(self.samples.len() as u64)
    }

    pub fn end(&self) -> Duration {
        self.start + self.duration()
    }
}

/// Esito dell'elaborazione di un frame.
#[derive(Debug, Clone)]
pub enum VadEvent {
    /// Inizio del parlato: utile per mostrare "sta parlando…" in UI.
    SpeechStarted { channel: Channel, at: Duration },
    /// Frase completa.
    Segment(SpeechSegment),
}

/// Segmentatore di un canale. Non è legato a un runtime: l'inferenza costa
/// meno di 1 ms per finestra da 32 ms, si può chiamare da un task async.
pub struct Segmenter {
    vad: VoiceActivityDetector,
    channel: Channel,
    padding: u64,
    /// Lunghezza massima di una frase emessa (padding incluso), in campioni.
    max_len: u64,
    history: History,
    /// Tempo del campione 0 (dal primo frame ricevuto).
    origin: Option<Duration>,
    /// Campioni passati al VAD finora.
    fed: u64,
    /// Fine (esclusa) dell'ultima frase emessa, per non sovrapporre il padding.
    last_end: u64,
    speaking: bool,
}

impl Segmenter {
    pub fn new(config: &VadConfig, channel: Channel) -> Result<Self, VadError> {
        if !config.model.is_file() {
            return Err(VadError::ModelNotFound(config.model.clone()));
        }
        let vad_config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(config.model.to_string_lossy().into_owned()),
                threshold: config.threshold,
                min_silence_duration: config.min_silence.as_secs_f32(),
                min_speech_duration: config.min_speech.as_secs_f32(),
                window_size: WINDOW,
                max_speech_duration: config.max_speech.as_secs_f32(),
            },
            sample_rate: TARGET_SAMPLE_RATE as i32,
            num_threads: 1,
            provider: Some("cpu".into()),
            ..Default::default()
        };
        // Il buffer interno deve contenere la frase più lunga.
        let buffer_secs = (config.max_speech + HISTORY_SLACK).as_secs_f32();
        let vad = VoiceActivityDetector::create(&vad_config, buffer_secs)
            .ok_or_else(|| VadError::Init(config.model.clone()))?;
        let history_len = duration_to_samples(
            config.max_speech + config.min_silence + config.padding * 2 + HISTORY_SLACK,
        );
        Ok(Self {
            vad,
            channel,
            padding: duration_to_samples(config.padding),
            max_len: duration_to_samples(config.max_speech + config.padding * 2).max(1),
            history: History::new(history_len as usize),
            origin: None,
            fed: 0,
            last_end: 0,
            speaking: false,
        })
    }

    pub fn channel(&self) -> Channel {
        self.channel
    }

    /// `true` se il VAD sta rilevando parlato adesso.
    pub fn is_speaking(&self) -> bool {
        self.speaking
    }

    /// Elabora un frame del proprio canale (gli altri vengono ignorati).
    pub fn push(&mut self, frame: &AudioFrame) -> Vec<VadEvent> {
        if frame.channel != self.channel || frame.samples.is_empty() {
            return Vec::new();
        }
        self.origin.get_or_insert(frame.start);
        self.history.push(&frame.samples);
        self.vad.accept_waveform(&frame.samples);
        self.fed += frame.samples.len() as u64;

        let mut events = Vec::new();
        let detected = self.vad.detected();
        if detected && !self.speaking {
            events.push(VadEvent::SpeechStarted {
                channel: self.channel,
                at: self.time_of(self.fed.saturating_sub(frame.samples.len() as u64)),
            });
        }
        self.speaking = detected;
        self.drain(&mut events);
        events
    }

    /// Fine audio: chiude l'eventuale frase in corso.
    pub fn flush(&mut self) -> Vec<VadEvent> {
        self.vad.flush();
        self.speaking = false;
        let mut events = Vec::new();
        self.drain(&mut events);
        events
    }

    fn drain(&mut self, events: &mut Vec<VadEvent>) {
        while let Some(seg) = self.vad.front() {
            let start = u64::try_from(seg.start()).unwrap_or(0);
            let end = start + seg.samples().len() as u64;
            let to = (end + self.padding).min(self.fed);
            // Mai prima della fine della frase precedente: se il suo padding
            // finale copre già l'inizio di questa, quei campioni restano lì.
            let from = start
                .saturating_sub(self.padding)
                .max(self.last_end)
                .max(self.history.start())
                .min(to);
            let samples = self
                .history
                .range(from, to)
                .unwrap_or_else(|| seg.samples().to_vec());
            let from = if samples.len() as u64 == to - from {
                from
            } else {
                start
            };
            self.vad.pop();
            if samples.is_empty() {
                continue;
            }
            self.last_end = from + samples.len() as u64;
            // Silero tratta `max_speech` come limite morbido: oltre il
            // massimo si taglia comunque, in parti uguali.
            let parts = (samples.len() as u64).div_ceil(self.max_len).max(1) as usize;
            let part_len = samples.len().div_ceil(parts).max(1);
            for (i, chunk) in samples.chunks(part_len).enumerate() {
                events.push(VadEvent::Segment(SpeechSegment {
                    channel: self.channel,
                    start: self.time_of(from + (i * part_len) as u64),
                    samples: chunk.to_vec(),
                }));
            }
        }
    }

    fn time_of(&self, sample: u64) -> Duration {
        self.origin.unwrap_or_default() + samples_to_duration(sample)
    }
}

fn samples_to_duration(n: u64) -> Duration {
    Duration::from_micros(n * 1_000_000 / RATE)
}

fn duration_to_samples(d: Duration) -> u64 {
    (d.as_micros() * u128::from(RATE) / 1_000_000) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_time_roundtrip() {
        assert_eq!(samples_to_duration(16_000), Duration::from_secs(1));
        assert_eq!(duration_to_samples(Duration::from_millis(200)), 3_200);
        assert_eq!(
            samples_to_duration(duration_to_samples(Duration::from_millis(1234))),
            Duration::from_millis(1234)
        );
    }

    #[test]
    fn missing_model_is_a_clear_error() {
        let err = Segmenter::new(&VadConfig::new("/nonexistent/silero_vad.onnx"), Channel::Me)
            .err()
            .unwrap();
        assert!(matches!(err, VadError::ModelNotFound(_)), "{err}");
    }
}
