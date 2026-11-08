//! Impronta vocale CAM++ (3D-Speaker) via `sherpa-onnx`.

use std::path::Path;

use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};
use sv_audio::TARGET_SAMPLE_RATE;

use crate::DiarError;
use crate::tracker::normalized;

/// Sotto questa durata (dopo il ritaglio del silenzio) l'impronta non vale.
const MIN_EMBED_S: f32 = 0.35;
/// Ritaglio del silenzio: margine attorno alla voce, soglia relativa al
/// picco e picco minimo (100 su int16 nel prototipo).
const TRIM_MARGIN_S: f32 = 0.1;
const TRIM_RATIO: f32 = 0.05;
const TRIM_MIN_PEAK: f32 = 100.0 / 32768.0;

/// Calcolo dell'impronta (vettore normalizzato) di una frase 16 kHz mono.
pub trait Embed {
    /// `None` se l'audio è troppo corto o muto.
    fn embed(&mut self, samples: &[f32]) -> Option<Vec<f32>>;
}

pub struct CampPlus {
    extractor: SpeakerEmbeddingExtractor,
}

impl CampPlus {
    pub fn load(model: &Path, num_threads: i32) -> Result<Self, DiarError> {
        if !model.is_file() {
            return Err(DiarError::ModelNotFound(model.to_owned()));
        }
        let config = SpeakerEmbeddingExtractorConfig {
            model: Some(model.to_string_lossy().into_owned()),
            num_threads,
            debug: false,
            provider: Some("cpu".into()),
        };
        let extractor = SpeakerEmbeddingExtractor::create(&config).ok_or(DiarError::Load)?;
        Ok(Self { extractor })
    }
}

impl Embed for CampPlus {
    fn embed(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
        let voiced = trim_silence(samples);
        if (voiced.len() as f32) < MIN_EMBED_S * TARGET_SAMPLE_RATE as f32 {
            return None;
        }
        let stream = self.extractor.create_stream()?;
        stream.accept_waveform(TARGET_SAMPLE_RATE as i32, voiced);
        stream.input_finished();
        if !self.extractor.is_ready(&stream) {
            return None;
        }
        normalized(&self.extractor.compute(&stream)?)
    }
}

/// Toglie il silenzio iniziale e finale, lasciando un piccolo margine.
pub fn trim_silence(samples: &[f32]) -> &[f32] {
    let peak = samples.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if peak < TRIM_MIN_PEAK {
        return &[];
    }
    let threshold = TRIM_MIN_PEAK.max(peak * TRIM_RATIO);
    let loud = |x: &f32| x.abs() >= threshold;
    let (Some(first), Some(last)) = (
        samples.iter().position(loud),
        samples.iter().rposition(loud),
    ) else {
        return &[];
    };
    let margin = (TRIM_MARGIN_S * TARGET_SAMPLE_RATE as f32) as usize;
    &samples[first.saturating_sub(margin)..(last + margin + 1).min(samples.len())]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_silence_with_margin() {
        let mut s = vec![0.0f32; 16_000];
        s[8_000..9_000].fill(0.5);
        let t = trim_silence(&s);
        assert_eq!(t.len(), 1_000 + 2 * 1_600);
        assert!(trim_silence(&[0.001; 1000]).is_empty());
        assert!(trim_silence(&[]).is_empty());
    }
}
