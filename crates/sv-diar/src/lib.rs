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
//! [`voices`] riconosce, se l'utente lo attiva, i parlanti già nominati in
//! meeting precedenti (profili vocali locali).
//!
//! [`AudioRing`] conserva l'audio recente del canale per ritagliare la frase
//! dai suoi tempi (`start`/`end`), qualunque sia il motore STT.

mod cluster;
mod embed;
mod ring;
mod tracker;
pub mod voices;

use std::collections::HashMap;

pub use cluster::cluster;
pub use embed::{CampPlus, Embed, trim_silence};
pub use ring::AudioRing;
pub use tracker::{SpeakerTracker, TrackerConfig};
pub use voices::{
    KnownVoice, MatchThresholds, Recognizer, SpeakerVoice, Verdict, VoiceMatch, VoiceStore,
};

/// Indice del parlante (0 = "Speaker 1").
pub type Speaker = u8;

/// Parlanti al massimo.
pub const MAX_SPEAKERS: usize = 8;

/// Frasi più corte non creano nuovi parlanti: ereditano o restano vuote.
/// Sotto i 2 s l'impronta CAM++ di una voce compressa (Teams, Meet) varia
/// troppo e una sola frase basta a creare un parlante fantasma.
pub const NEW_SPEAKER_MIN_S: f32 = 2.0;

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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Assignment {
    pub speaker: Option<Speaker>,
    /// Frasi precedenti che cambiano parlante (solo a numero fissato).
    pub relabels: Vec<(u64, Option<Speaker>)>,
    /// Parlanti riconosciuti nei profili vocali (solo le novità).
    pub voices: Vec<VoiceMatch>,
}

/// Attribuisce le frasi ai parlanti. Uno per sessione, un thread alla volta.
pub struct Diarizer<E: Embed> {
    embedder: E,
    mode: Mode,
    recognizer: Option<Recognizer>,
}

enum Mode {
    Auto {
        tracker: SpeakerTracker,
        /// Per parlante: somma delle impronte pesate per durata e secondi.
        sums: Vec<(Vec<f32>, f32)>,
    },
    Fixed {
        k: usize,
        /// (id frase, impronta, durata in s) delle frasi con impronta valida.
        history: Vec<(u64, Vec<f32>, f32)>,
        /// Parlante corrente di ogni frase in `history`.
        labels: HashMap<u64, Speaker>,
        last: Option<Speaker>,
    },
}

impl<E: Embed> Diarizer<E> {
    /// `speakers` = numero di parlanti noto; 0 = automatico.
    pub fn new(embedder: E, speakers: u8) -> Self {
        let mode = match usize::from(speakers).min(MAX_SPEAKERS) {
            0 => Mode::Auto {
                tracker: SpeakerTracker::new(TrackerConfig::default()),
                sums: Vec::new(),
            },
            k => Mode::Fixed {
                k,
                history: Vec::new(),
                labels: HashMap::new(),
                last: None,
            },
        };
        Self {
            embedder,
            mode,
            recognizer: None,
        }
    }

    /// Riconosce i parlanti nei profili vocali `known`.
    pub fn with_voices(mut self, known: Vec<KnownVoice>, thresholds: MatchThresholds) -> Self {
        let recognizer = Recognizer::new(known, thresholds);
        self.recognizer = (!recognizer.is_empty()).then_some(recognizer);
        self
    }

    /// Voce di ogni parlante nella sessione: media delle impronte delle sue
    /// frasi da almeno [`voices::MIN_SEGMENT_S`], pesata per durata.
    pub fn voices(&self) -> Vec<SpeakerVoice> {
        let sums: Vec<(Speaker, Vec<f32>, f32)> = match &self.mode {
            Mode::Auto { sums, .. } => sums
                .iter()
                .enumerate()
                .filter_map(|(i, (sum, s))| Some((Speaker::try_from(i).ok()?, sum.clone(), *s)))
                .collect(),
            Mode::Fixed {
                history, labels, ..
            } => {
                let mut by: Vec<(Speaker, Vec<f32>, f32)> = Vec::new();
                for (id, v, d) in history {
                    let (Some(speaker), true) = (labels.get(id), *d >= voices::MIN_SEGMENT_S)
                    else {
                        continue;
                    };
                    let i = match by.iter().position(|(s, _, _)| s == speaker) {
                        Some(i) => i,
                        None => {
                            by.push((*speaker, vec![0.0; v.len()], 0.0));
                            by.len() - 1
                        }
                    };
                    let (_, sum, seconds) = &mut by[i];
                    accumulate(sum, seconds, v, *d);
                }
                by.sort_by_key(|(s, _, _)| *s);
                by
            }
        };
        sums.into_iter()
            .filter(|(_, _, s)| *s > 0.0)
            .filter_map(|(speaker, sum, seconds)| {
                Some(SpeakerVoice {
                    speaker,
                    centroid: tracker::normalized(&sum)?,
                    seconds,
                })
            })
            .collect()
    }

    /// Attribuisce la frase `segment_id` (audio 16 kHz mono) a un parlante.
    pub fn assign(&mut self, segment_id: u64, samples: &[f32]) -> Assignment {
        let duration = samples.len() as f32 / sv_audio::TARGET_SAMPLE_RATE as f32;
        let vector = self.embedder.embed(samples);
        let embedded = vector.is_some();
        let mut found = self.place(segment_id, vector, duration);
        if embedded && duration >= voices::MIN_SEGMENT_S && self.recognizer.is_some() {
            let current = self.voices();
            if let Some(r) = &mut self.recognizer {
                found.voices = r.update(&current);
            }
        }
        found
    }

    /// Frase il cui parlante è già noto (diarization del motore STT, es.
    /// Deepgram): l'impronta serve solo alla voce del parlante e ai profili
    /// vocali. A numero di parlanti fissato l'etichetta viene restituita così.
    pub fn assign_labeled(&mut self, samples: &[f32], speaker: Speaker) -> Assignment {
        let duration = samples.len() as f32 / sv_audio::TARGET_SAMPLE_RATE as f32;
        let mut found = Assignment {
            speaker: Some(speaker),
            ..Assignment::default()
        };
        let Mode::Auto { sums, .. } = &mut self.mode else {
            return found;
        };
        if duration < voices::MIN_SEGMENT_S {
            return found;
        }
        let Some(v) = self
            .embedder
            .embed(samples)
            .as_deref()
            .and_then(tracker::normalized)
        else {
            return found;
        };
        let i = usize::from(speaker);
        if sums.len() <= i {
            sums.resize(i + 1, (vec![0.0; v.len()], 0.0));
        }
        let (sum, seconds) = &mut sums[i];
        if sum.len() == v.len() {
            accumulate(sum, seconds, &v, duration);
        }
        if self.recognizer.is_some() {
            let current = self.voices();
            if let Some(r) = &mut self.recognizer {
                found.voices = r.update(&current);
            }
        }
        found
    }

    fn place(&mut self, segment_id: u64, vector: Option<Vec<f32>>, duration: f32) -> Assignment {
        match &mut self.mode {
            Mode::Auto { tracker, sums } => {
                let speaker = tracker.assign(vector.as_deref(), duration);
                if let (Some(s), Some(v)) =
                    (speaker, vector.as_deref().and_then(tracker::normalized))
                    && duration >= voices::MIN_SEGMENT_S
                {
                    let i = usize::from(s);
                    if sums.len() <= i {
                        sums.resize(i + 1, (vec![0.0; v.len()], 0.0));
                    }
                    let (sum, seconds) = &mut sums[i];
                    accumulate(sum, seconds, &v, duration);
                }
                Assignment {
                    speaker,
                    ..Assignment::default()
                }
            }
            Mode::Fixed {
                k,
                history,
                labels,
                last,
            } => {
                let Some(vector) = vector else {
                    return Assignment {
                        speaker: *last,
                        ..Assignment::default()
                    };
                };
                if history.len() == MAX_HISTORY {
                    let (old, _, _) = history.remove(0);
                    labels.remove(&old);
                }
                history.push((segment_id, vector, duration));
                let vectors: Vec<&[f32]> = history.iter().map(|(_, v, _)| v.as_slice()).collect();
                let assigned = cluster(&vectors, *k);
                let mut relabels = Vec::new();
                for ((id, _, _), speaker) in history.iter().zip(&assigned) {
                    let prev = labels.insert(*id, *speaker);
                    if *id != segment_id && prev != Some(*speaker) {
                        relabels.push((*id, Some(*speaker)));
                    }
                }
                let speaker = assigned.last().copied();
                *last = speaker;
                Assignment {
                    speaker,
                    relabels,
                    voices: Vec::new(),
                }
            }
        }
    }
}

/// Somma un'impronta (normalizzata) pesata per `duration`.
fn accumulate(sum: &mut [f32], seconds: &mut f32, v: &[f32], duration: f32) {
    if sum.len() != v.len() {
        return;
    }
    sum.iter_mut().zip(v).for_each(|(s, x)| *s += x * duration);
    *seconds += duration;
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
                    ..Assignment::default()
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

    fn known(id: &str, axis: usize) -> KnownVoice {
        KnownVoice {
            id: id.into(),
            centroid: voice(axis, 0.0, 0),
        }
    }

    #[test]
    fn auto_mode_recognizes_known_voice_after_enough_speech() {
        // Voce A (asse 0) è "marco"; voce B (asse 1) è nuova.
        let axes = [0, 1, 0, 1, 0];
        let script = axes
            .iter()
            .enumerate()
            .map(|(i, &a)| Some(voice(a, 0.2, i as u32)))
            .collect();
        let mut d = Diarizer::new(Scripted(script), 0)
            .with_voices(vec![known("marco", 0)], MatchThresholds::default());
        // 3 s a frase: Marco supera i 6 s alla seconda frase sua (id 3).
        let found: Vec<_> = (1..=5).map(|id| d.assign(id, &secs(3.0)).voices).collect();
        assert!(found[0].is_empty() && found[1].is_empty());
        assert_eq!(found[2].len(), 1);
        assert_eq!(found[2][0].speaker, 0);
        assert_eq!(found[2][0].profile, "marco");
        assert_eq!(found[2][0].verdict, Verdict::Assign);
        // Nessun altro annuncio: né per Marco né per la voce nuova.
        assert!(found[3].is_empty() && found[4].is_empty());

        let voices = d.voices();
        assert_eq!(voices.len(), 2);
        assert_eq!(voices[0].seconds, 9.0);
        assert_eq!(voices[1].seconds, 6.0);
    }

    #[test]
    fn fixed_mode_recognizes_known_voice() {
        let axes = [1, 0, 1, 0];
        let script = axes
            .iter()
            .enumerate()
            .map(|(i, &a)| Some(voice(a, 0.2, i as u32)))
            .collect();
        let mut d = Diarizer::new(Scripted(script), 2)
            .with_voices(vec![known("anna", 0)], MatchThresholds::default());
        let all: Vec<_> = (1..=4)
            .flat_map(|id| d.assign(id, &secs(4.0)).voices)
            .collect();
        assert_eq!(all.len(), 1);
        // Anna parla per seconda: "Speaker 2".
        assert_eq!((all[0].speaker, all[0].profile.as_str()), (1, "anna"));
    }

    #[test]
    fn short_segments_do_not_count_as_speech() {
        let script = (0..4).map(|i| Some(voice(0, 0.2, i))).collect();
        let mut d = Diarizer::new(Scripted(script), 0)
            .with_voices(vec![known("marco", 0)], MatchThresholds::default());
        // Prima frase lunga crea il parlante, poi frasi brevi: niente parlato.
        assert!(d.assign(1, &secs(2.0)).voices.is_empty());
        for id in 2..=4 {
            assert!(d.assign(id, &secs(0.8)).voices.is_empty());
        }
        assert_eq!(d.voices()[0].seconds, 2.0);
    }

    #[test]
    fn fixed_mode_short_segment_inherits() {
        let mut d = Diarizer::new(Scripted(vec![Some(voice(3, 0.1, 1)), None].into()), 2);
        assert_eq!(d.assign(1, &secs(2.0)).speaker, Some(0));
        assert_eq!(d.assign(2, &secs(0.2)).speaker, Some(0));
    }
}
