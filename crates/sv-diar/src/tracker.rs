//! Attribuzione online delle impronte ai parlanti già visti.

use crate::{MAX_SPEAKERS, NEW_SPEAKER_MIN_S, Speaker};

#[derive(Debug, Clone)]
pub struct TrackerConfig {
    /// Similarità coseno minima per "stesso parlante". Con CAM++ la stessa
    /// voce su frasi da 1 s sta sopra 0.46, voci diverse sotto 0.35.
    pub threshold: f32,
    /// Sotto questa similarità con tutti i profili nasce un parlante nuovo.
    /// Tra `new_speaker` e `threshold` la frase va al profilo più vicino
    /// senza aggiornarlo: con l'audio compresso delle call la stessa voce
    /// scende spesso sotto `threshold`, e dividerla è peggio che unire.
    pub new_speaker: f32,
    /// Sotto questa durata (s) la soglia scende di `short_bonus`: una parola
    /// isolata non crea un parlante fantasma.
    pub short_utterance: f32,
    pub short_bonus: f32,
    /// Sotto questa durata (s) l'impronta è troppo instabile per aggiornare
    /// il profilo: la frase viene solo etichettata.
    pub min_update: f32,
    pub max_speakers: usize,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            threshold: 0.45,
            new_speaker: 0.35,
            short_utterance: 1.2,
            short_bonus: 0.05,
            min_update: 0.4,
            max_speakers: MAX_SPEAKERS,
        }
    }
}

/// Profili (centroidi normalizzati) dei parlanti visti finora.
pub struct SpeakerTracker {
    config: TrackerConfig,
    centroids: Vec<Vec<f32>>,
    counts: Vec<u32>,
    last: Option<Speaker>,
}

impl SpeakerTracker {
    pub fn new(config: TrackerConfig) -> Self {
        Self {
            config,
            centroids: Vec::new(),
            counts: Vec::new(),
            last: None,
        }
    }

    pub fn speakers(&self) -> usize {
        self.centroids.len()
    }

    /// Parlante della frase (`duration` in secondi). Senza impronta, o se la
    /// frase è troppo corta per crearne uno nuovo, eredita il precedente.
    pub fn assign(&mut self, vector: Option<&[f32]>, duration: f32) -> Option<Speaker> {
        let Some(v) = vector.and_then(normalized) else {
            return self.last;
        };
        let best = self
            .centroids
            .iter()
            .enumerate()
            .map(|(i, c)| (i, dot(&v, c)))
            .max_by(|a, b| a.1.total_cmp(&b.1));

        let (mut threshold, mut new_speaker) = (self.config.threshold, self.config.new_speaker);
        if duration < self.config.short_utterance {
            threshold -= self.config.short_bonus;
            new_speaker -= self.config.short_bonus;
        }
        let index = match best {
            Some((i, sim)) if sim >= threshold => i,
            // Zona grigia: profilo più vicino, ma l'impronta non lo sporca.
            Some((i, sim)) if sim >= new_speaker => {
                let speaker = Speaker::try_from(i).ok();
                self.last = speaker;
                return speaker;
            }
            _ if duration < NEW_SPEAKER_MIN_S => return self.last,
            _ if self.centroids.len() < self.config.max_speakers => {
                self.centroids.push(v.clone());
                self.counts.push(0);
                self.centroids.len() - 1
            }
            Some((i, _)) => i,
            None => return self.last,
        };
        if duration >= self.config.min_update {
            self.update(index, &v);
        }
        let speaker = Speaker::try_from(index).ok();
        self.last = speaker;
        speaker
    }

    fn update(&mut self, index: usize, v: &[f32]) {
        let count = self.counts[index];
        let c = &mut self.centroids[index];
        if count == 0 {
            c.copy_from_slice(v);
        } else if count < 3 {
            let n = count as f32;
            for (a, b) in c.iter_mut().zip(v) {
                *a = (*a * n + b) / (n + 1.0);
            }
        } else {
            // Media mobile: il profilo si adatta lentamente alla voce.
            for (a, b) in c.iter_mut().zip(v) {
                *a = 0.8 * *a + 0.2 * b;
            }
        }
        if let Some(n) = normalized(c) {
            *c = n;
        }
        self.counts[index] = count + 1;
    }
}

pub(crate) fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub(crate) fn normalized(v: &[f32]) -> Option<Vec<f32>> {
    let norm = dot(v, v).sqrt();
    (norm > 0.0 && norm.is_finite()).then(|| v.iter().map(|x| x / norm).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::voice;

    #[test]
    fn same_voice_same_speaker_new_voice_new_speaker() {
        let mut t = SpeakerTracker::new(TrackerConfig::default());
        assert_eq!(t.assign(Some(&voice(0, 0.3, 1)), 2.0), Some(0));
        assert_eq!(t.assign(Some(&voice(0, 0.3, 2)), 2.0), Some(0));
        assert_eq!(t.assign(Some(&voice(1, 0.3, 3)), 2.0), Some(1));
        assert_eq!(t.assign(Some(&voice(0, 0.3, 4)), 2.0), Some(0));
        assert_eq!(t.speakers(), 2);
    }

    #[test]
    fn short_segments_never_create_speakers() {
        let mut t = SpeakerTracker::new(TrackerConfig::default());
        // Prima frase corta e nessun parlante: resta vuoto.
        assert_eq!(t.assign(Some(&voice(0, 0.1, 1)), 0.6), None);
        assert_eq!(t.assign(Some(&voice(0, 0.1, 2)), 2.0), Some(0));
        // Voce nuova ma frase corta: eredita il precedente.
        assert_eq!(t.assign(Some(&voice(1, 0.1, 3)), 0.8), Some(0));
        assert_eq!(t.speakers(), 1);
        // Senza impronta (audio troppo corto o muto): eredita.
        assert_eq!(t.assign(None, 0.2), Some(0));
        assert_eq!(t.assign(Some(&[0.0; 16]), 2.0), Some(0));
    }

    #[test]
    fn grey_zone_joins_closest_without_new_speaker() {
        let mut t = SpeakerTracker::new(TrackerConfig::default());
        let a = voice(0, 0.0, 1);
        assert_eq!(t.assign(Some(&a), 3.0), Some(0));
        // Similarità ~0.40: sotto `threshold`, sopra `new_speaker`.
        let mut b = vec![0.0; a.len()];
        b[0] = 0.4;
        b[1] = (1.0f32 - 0.16).sqrt();
        assert!((dot(&normalized(&b).unwrap(), &normalized(&a).unwrap()) - 0.4).abs() < 0.05);
        assert_eq!(t.assign(Some(&b), 3.0), Some(0));
        assert_eq!(t.speakers(), 1);
    }

    #[test]
    fn caps_speakers_and_falls_back_to_closest() {
        let mut t = SpeakerTracker::new(TrackerConfig {
            max_speakers: 2,
            ..TrackerConfig::default()
        });
        assert_eq!(t.assign(Some(&voice(0, 0.0, 1)), 2.0), Some(0));
        assert_eq!(t.assign(Some(&voice(1, 0.0, 2)), 2.0), Some(1));
        let mut third = voice(2, 0.0, 3);
        third[1] = 0.3;
        assert_eq!(t.assign(Some(&third), 2.0), Some(1));
        assert_eq!(t.speakers(), 2);
    }
}
