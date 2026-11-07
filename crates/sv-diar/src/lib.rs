//! Riconoscimento dei parlanti (diarization) sul canale "Loro".
//!
//! Port di `diarization.py` del prototipo. Per ogni frase finale si calcola
//! un'impronta della voce (CAM++, via `sherpa-onnx`) e la si attribuisce a un
//! parlante:
//!
//! - numero di parlanti libero: [`SpeakerTracker`] confronta l'impronta con i
//!   profili già visti (similarità coseno) e ne crea uno nuovo sopra soglia;
//! - numero fissato dall'utente: [`cluster`] (k-means sferico) raggruppa
//!   tutte le impronte a ogni frase, e le frasi già mostrate possono cambiare
//!   parlante (rietichettatura).
//!
//! È una diarization a posteriori, frase per frase: due persone che parlano
//! senza pausa finiscono nella stessa frase.
//!
//! [`AudioRing`] conserva l'audio recente del canale per ritagliare la frase
//! dai suoi tempi (`start`/`end`), qualunque sia il motore STT.

mod cluster;
mod embed;
mod ring;
mod tracker;

use std::collections::HashMap;

pub use cluster::cluster;
pub use embed::{CampPlus, Embed, trim_silence};
pub use ring::AudioRing;
pub use tracker::{SpeakerTracker, TrackerConfig};

/// Indice del parlante (0 = "Speaker 1").
pub type Speaker = u8;

/// Parlanti al massimo.
pub const MAX_SPEAKERS: usize = 8;

/// Frasi più corte non creano nuovi parlanti: ereditano o restano vuote.
pub const NEW_SPEAKER_MIN_S: f32 = 1.0;

/// Impronte conservate per il raggruppamento a numero fissato.
const MAX_HISTORY: usize = 1000;

#[derive(Debug, Clone, thiserror::Error)]
pub enum DiarError {
    #[error("modello CAM++ non trovato: {0}")]
    ModelNotFound(std::path::PathBuf),
    #[error("impossibile caricare il modello CAM++")]
    Load,
}

/// Esito per una frase.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assignment {
    pub speaker: Option<Speaker>,
    /// Frasi precedenti che cambiano parlante (solo a numero fissato).
    pub relabels: Vec<(u64, Option<Speaker>)>,
}

/// Attribuisce le frasi ai parlanti. Uno per sessione, un thread alla volta.
pub struct Diarizer<E: Embed> {
    embedder: E,
    mode: Mode,
}

enum Mode {
    Auto(SpeakerTracker),
    Fixed {
        k: usize,
        /// (id frase, impronta) delle frasi con impronta valida.
        history: Vec<(u64, Vec<f32>)>,
        /// Parlante corrente di ogni frase in `history`.
        labels: HashMap<u64, Speaker>,
        last: Option<Speaker>,
    },
}

impl<E: Embed> Diarizer<E> {
    /// `speakers` = numero di parlanti noto; 0 = automatico.
    pub fn new(embedder: E, speakers: u8) -> Self {
        let mode = match usize::from(speakers).min(MAX_SPEAKERS) {
            0 => Mode::Auto(SpeakerTracker::new(TrackerConfig::default())),
            k => Mode::Fixed {
                k,
                history: Vec::new(),
                labels: HashMap::new(),
                last: None,
            },
        };
        Self { embedder, mode }
    }

    /// Attribuisce la frase `segment_id` (audio 16 kHz mono) a un parlante.
    pub fn assign(&mut self, segment_id: u64, samples: &[f32]) -> Assignment {
        let duration = samples.len() as f32 / sv_audio::TARGET_SAMPLE_RATE as f32;
        let vector = self.embedder.embed(samples);
        match &mut self.mode {
            Mode::Auto(tracker) => Assignment {
                speaker: tracker.assign(vector.as_deref(), duration),
                relabels: Vec::new(),
            },
            Mode::Fixed {
                k,
                history,
                labels,
                last,
            } => {
                let Some(vector) = vector else {
                    return Assignment {
                        speaker: *last,
                        relabels: Vec::new(),
                    };
                };
                if history.len() == MAX_HISTORY {
                    let (old, _) = history.remove(0);
                    labels.remove(&old);
                }
                history.push((segment_id, vector));
                let vectors: Vec<&[f32]> = history.iter().map(|(_, v)| v.as_slice()).collect();
                let assigned = cluster(&vectors, *k);
                let mut relabels = Vec::new();
                for ((id, _), speaker) in history.iter().zip(&assigned) {
                    let prev = labels.insert(*id, *speaker);
                    if *id != segment_id && prev != Some(*speaker) {
                        relabels.push((*id, Some(*speaker)));
                    }
                }
                let speaker = assigned.last().copied();
                *last = speaker;
                Assignment { speaker, relabels }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    /// Vettore unitario "attorno" all'asse `axis`, con un po' di rumore
    /// deterministico.
    pub fn voice(axis: usize, noise: f32, seed: u32) -> Vec<f32> {
        let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(axis as u32);
        let mut v: Vec<f32> = (0..16)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let r = (state % 1000) as f32 / 1000.0 - 0.5;
                let base = if i == axis { 1.0 } else { 0.0 };
                base + noise * r
            })
            .collect();
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter_mut().for_each(|x| *x /= n);
        v
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::testutil::voice;
    use super::*;

    /// Embedder finto: restituisce le impronte preparate, in ordine.
    struct Scripted(VecDeque<Option<Vec<f32>>>);

    impl Embed for Scripted {
        fn embed(&mut self, _samples: &[f32]) -> Option<Vec<f32>> {
            self.0.pop_front().flatten()
        }
    }

    fn secs(s: f32) -> Vec<f32> {
        vec![0.0; (s * 16_000.0) as usize]
    }

    #[test]
    fn auto_mode_tracks_two_voices() {
        let script = [0, 1, 0, 1, 1, 0]
            .iter()
            .enumerate()
            .map(|(i, &a)| Some(voice(a, 0.3, i as u32)))
            .collect();
        let mut d = Diarizer::new(Scripted(script), 0);
        let got: Vec<_> = (1..=6).map(|id| d.assign(id, &secs(2.0)).speaker).collect();
        assert_eq!(got, [0, 1, 0, 1, 1, 0].map(Some));
    }

    #[test]
    fn fixed_mode_relabels_earlier_segments() {
        // Due voci, k = 1: tutto Speaker 1, nessuna rietichettatura.
        let script = [0, 1, 0]
            .iter()
            .enumerate()
            .map(|(i, &a)| Some(voice(a, 0.2, i as u32)))
            .collect();
        let mut d = Diarizer::new(Scripted(script), 1);
        for id in 1..=3 {
            assert_eq!(
                d.assign(id, &secs(2.0)),
                Assignment {
                    speaker: Some(0),
                    relabels: vec![]
                }
            );
        }

        // k = 2 con la stessa voce due volte e poi un'altra: la seconda
        // frase, prima messa a parte, torna con la prima.
        let script = [0, 0, 1]
            .iter()
            .enumerate()
            .map(|(i, &a)| Some(voice(a, 0.2, i as u32)))
            .collect();
        let mut d = Diarizer::new(Scripted(script), 2);
        assert_eq!(d.assign(1, &secs(2.0)).speaker, Some(0));
        assert_eq!(d.assign(2, &secs(2.0)).speaker, Some(1));
        let third = d.assign(3, &secs(2.0));
        assert_eq!(third.speaker, Some(1));
        assert_eq!(third.relabels, vec![(2, Some(0))]);
    }

    #[test]
    fn fixed_mode_short_segment_inherits() {
        let mut d = Diarizer::new(Scripted(vec![Some(voice(3, 0.1, 1)), None].into()), 2);
        assert_eq!(d.assign(1, &secs(2.0)).speaker, Some(0));
        assert_eq!(d.assign(2, &secs(0.2)).speaker, Some(0));
    }
}
