/// Pavimento in dBFS per il silenzio digitale.
pub const FLOOR_DB: f32 = -100.0;

/// Accumula campioni mono e produce (rms_db, peak_db) a finestre fisse.
pub struct LevelMeter {
    window: usize,
    count: usize,
    sum_sq: f64,
    peak: f32,
}

impl LevelMeter {
    /// `window` = numero di campioni per finestra (es. 50 ms alla frequenza nativa).
    pub fn new(window: usize) -> Self {
        Self {
            window: window.max(1),
            count: 0,
            sum_sq: 0.0,
            peak: 0.0,
        }
    }

    /// Aggiunge campioni; chiama `emit` a ogni finestra completata.
    pub fn push(&mut self, samples: &[f32], mut emit: impl FnMut(f32, f32)) {
        for &s in samples {
            self.sum_sq += f64::from(s) * f64::from(s);
            self.peak = self.peak.max(s.abs());
            self.count += 1;
            if self.count == self.window {
                let rms = (self.sum_sq / self.count as f64).sqrt() as f32;
                emit(to_db(rms), to_db(self.peak));
                self.count = 0;
                self.sum_sq = 0.0;
                self.peak = 0.0;
            }
        }
    }
}

pub fn to_db(amplitude: f32) -> f32 {
    if amplitude <= 1e-5 {
        FLOOR_DB
    } else {
        (20.0 * amplitude.log10()).max(FLOOR_DB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_floor() {
        let mut m = LevelMeter::new(4);
        let mut out = vec![];
        m.push(&[0.0; 8], |r, p| out.push((r, p)));
        assert_eq!(out, vec![(FLOOR_DB, FLOOR_DB); 2]);
    }

    #[test]
    fn full_scale_square_is_zero_db() {
        let mut m = LevelMeter::new(4);
        let mut out = vec![];
        m.push(&[1.0, -1.0, 1.0, -1.0], |r, p| out.push((r, p)));
        assert_eq!(out.len(), 1);
        assert!(out[0].0.abs() < 1e-4);
        assert!(out[0].1.abs() < 1e-4);
    }

    #[test]
    fn half_amplitude_is_minus_six_db() {
        assert!((to_db(0.5) + 6.0206).abs() < 1e-3);
    }
}
