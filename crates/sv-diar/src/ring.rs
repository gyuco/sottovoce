//! Audio recente di un canale, per ritagliare una frase dai suoi tempi.

use std::collections::VecDeque;
use std::time::Duration;

use sv_audio::{AudioFrame, TARGET_SAMPLE_RATE};

/// Buffer circolare a 16 kHz indicizzato dal tempo di cattura. I buchi tra
/// frame (frame persi) diventano silenzio, così gli indici restano allineati.
pub struct AudioRing {
    /// Indice assoluto (campioni dall'avvio) del primo campione in `buf`.
    first: u64,
    buf: VecDeque<f32>,
    capacity: usize,
}

impl AudioRing {
    pub fn new(window: Duration) -> Self {
        let capacity = to_index(window) as usize;
        Self {
            first: 0,
            buf: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn end(&self) -> u64 {
        self.first + self.buf.len() as u64
    }

    pub fn push(&mut self, frame: &AudioFrame) {
        let at = to_index(frame.start);
        let end = self.end();
        let samples: &[f32] = if at >= end {
            let gap = (at - end).min(self.capacity as u64) as usize;
            if self.buf.is_empty() {
                self.first = at;
            } else {
                self.buf.extend(std::iter::repeat_n(0.0, gap));
                self.first = at - self.buf.len() as u64;
            }
            &frame.samples
        } else {
            // Sovrapposizione (frame ripetuto o in ritardo): solo la coda nuova.
            let skip = ((end - at) as usize).min(frame.samples.len());
            &frame.samples[skip..]
        };
        self.buf.extend(samples);
        let excess = self.buf.len().saturating_sub(self.capacity);
        self.buf.drain(..excess);
        self.first += excess as u64;
    }

    /// Campioni in `[start, end)`; la parte già uscita dal buffer o non
    /// ancora arrivata manca.
    pub fn slice(&self, start: Duration, end: Duration) -> Vec<f32> {
        let from = to_index(start).max(self.first);
        let to = to_index(end).min(self.end());
        if from >= to {
            return Vec::new();
        }
        self.buf
            .range((from - self.first) as usize..(to - self.first) as usize)
            .copied()
            .collect()
    }

    /// Tempo fino a cui c'è audio.
    pub fn end_time(&self) -> Duration {
        Duration::from_micros(self.end() * 1_000_000 / u64::from(TARGET_SAMPLE_RATE))
    }
}

fn to_index(t: Duration) -> u64 {
    (t.as_micros() * u128::from(TARGET_SAMPLE_RATE) / 1_000_000) as u64
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sv_audio::Channel;

    use super::*;

    fn frame(start_ms: u64, len: usize, value: f32) -> AudioFrame {
        AudioFrame {
            channel: Channel::Them,
            start: Duration::from_millis(start_ms),
            samples: Arc::from(vec![value; len]),
        }
    }

    #[test]
    fn slices_by_time_and_fills_gaps() {
        let mut r = AudioRing::new(Duration::from_secs(10));
        r.push(&frame(0, 1600, 1.0)); // 0-100 ms
        r.push(&frame(200, 1600, 2.0)); // buco 100-200 ms
        assert_eq!(r.end_time(), Duration::from_millis(300));
        let s = r.slice(Duration::from_millis(50), Duration::from_millis(250));
        assert_eq!(s.len(), 3200);
        assert_eq!(s[0], 1.0);
        assert_eq!(s[1600], 0.0);
        assert_eq!(s[3199], 2.0);
        // Oltre la fine: tagliato.
        assert_eq!(
            r.slice(Duration::from_millis(250), Duration::from_secs(5))
                .len(),
            800
        );
    }

    #[test]
    fn keeps_only_the_window_and_skips_overlap() {
        let mut r = AudioRing::new(Duration::from_millis(200));
        for i in 0..5 {
            r.push(&frame(i * 100, 1600, i as f32));
        }
        assert!(
            r.slice(Duration::ZERO, Duration::from_millis(250))
                .is_empty()
        );
        let tail = r.slice(Duration::from_millis(300), Duration::from_millis(500));
        assert_eq!(tail.len(), 3200);
        assert_eq!((tail[0], tail[3199]), (3.0, 4.0));
        // Frame sovrapposto: solo la parte nuova.
        r.push(&frame(450, 1600, 9.0));
        assert_eq!(r.end_time(), Duration::from_millis(550));
        assert_eq!(
            r.slice(Duration::from_millis(450), Duration::from_millis(500))[0],
            4.0
        );
    }
}
