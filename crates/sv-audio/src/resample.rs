use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

use crate::error::AudioError;

/// Frame d'ingresso per chiamata al resampler (alla frequenza nativa).
const CHUNK: usize = 1024;

/// Resampler mono a blocchi: accumula input e restituisce output a 16 kHz
/// appena c'è un blocco completo. Se le frequenze coincidono, passa i dati.
pub struct MonoResampler {
    inner: Option<Fft<f32>>,
    pending: Vec<f32>,
    out: Vec<f32>,
}

impl MonoResampler {
    pub fn new(input_rate: u32, output_rate: u32) -> Result<Self, AudioError> {
        let inner = if input_rate == output_rate {
            None
        } else {
            Some(
                Fft::<f32>::new(
                    input_rate as usize,
                    output_rate as usize,
                    CHUNK,
                    1,
                    FixedSync::Input,
                )
                .map_err(|e| AudioError::Resampler(e.to_string()))?,
            )
        };
        let out_max = inner.as_ref().map_or(0, |r| r.output_frames_max());
        Ok(Self {
            inner,
            pending: Vec::with_capacity(CHUNK * 2),
            out: vec![0.0; out_max],
        })
    }

    /// Aggiunge campioni e chiama `emit` per ogni blocco ricampionato.
    pub fn push(&mut self, samples: &[f32], mut emit: impl FnMut(&[f32])) {
        let Some(rs) = self.inner.as_mut() else {
            if !samples.is_empty() {
                emit(samples);
            }
            return;
        };
        self.pending.extend_from_slice(samples);
        loop {
            let need = rs.input_frames_next();
            if self.pending.len() < need {
                break;
            }
            let produced = {
                let input = InterleavedSlice::new(&self.pending[..need], 1, need)
                    .expect("dimensioni input coerenti");
                let out_len = self.out.len();
                let mut output = InterleavedSlice::new_mut(&mut self.out, 1, out_len)
                    .expect("dimensioni output coerenti");
                match rs.process_into_buffer(&input, &mut output, None) {
                    Ok((_, written)) => written,
                    Err(e) => {
                        tracing::warn!("resampler: {e}");
                        0
                    }
                }
            };
            self.pending.drain(..need);
            if produced > 0 {
                emit(&self.out[..produced]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsamples_48k_to_16k() {
        let mut rs = MonoResampler::new(48_000, 16_000).unwrap();
        let input: Vec<f32> = (0..48_000)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5)
            .collect();
        let mut total = 0;
        rs.push(&input, |b| total += b.len());
        // Un secondo in ingresso ≈ 16000 campioni in uscita (meno il blocco ancora in coda).
        assert!((15_000..=16_000).contains(&total), "total = {total}");
    }

    #[test]
    fn passthrough_when_rates_match() {
        let mut rs = MonoResampler::new(16_000, 16_000).unwrap();
        let mut total = 0;
        rs.push(&[0.1; 320], |b| total += b.len());
        assert_eq!(total, 320);
    }
}
