//! Storia recente dei campioni di un canale, indicizzata in assoluto
//! (campioni dall'inizio), per ritagliare le frasi con il padding.

use std::collections::VecDeque;

pub(crate) struct History {
    buf: VecDeque<f32>,
    capacity: usize,
    /// Indice assoluto di `buf[0]`.
    start: u64,
}

impl History {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(capacity),
            capacity,
            start: 0,
        }
    }

    /// Indice assoluto del campione più vecchio conservato.
    pub(crate) fn start(&self) -> u64 {
        self.start
    }

    /// Indice assoluto successivo all'ultimo campione.
    pub(crate) fn end(&self) -> u64 {
        self.start + self.buf.len() as u64
    }

    pub(crate) fn push(&mut self, samples: &[f32]) {
        self.buf.extend(samples);
        let excess = self.buf.len().saturating_sub(self.capacity);
        if excess > 0 {
            self.buf.drain(..excess);
            self.start += excess as u64;
        }
    }

    /// Campioni `[from, to)`, se ancora tutti in memoria.
    pub(crate) fn range(&self, from: u64, to: u64) -> Option<Vec<f32>> {
        if from > to || from < self.start || to > self.end() {
            return None;
        }
        let a = (from - self.start) as usize;
        let b = (to - self.start) as usize;
        Some(self.buf.range(a..b).copied().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(from: u64, n: usize) -> Vec<f32> {
        (0..n).map(|i| (from + i as u64) as f32).collect()
    }

    #[test]
    fn keeps_last_capacity_samples_with_absolute_index() {
        let mut h = History::new(10);
        h.push(&ramp(0, 6));
        h.push(&ramp(6, 6));
        assert_eq!((h.start(), h.end()), (2, 12));
        assert_eq!(h.range(4, 7).unwrap(), vec![4.0, 5.0, 6.0]);
        assert_eq!(h.range(2, 12).unwrap().len(), 10);
    }

    #[test]
    fn rejects_ranges_outside_memory() {
        let mut h = History::new(4);
        h.push(&ramp(0, 8));
        assert!(h.range(3, 5).is_none(), "già scartati");
        assert!(h.range(6, 9).is_none(), "nel futuro");
        assert!(h.range(6, 5).is_none(), "invertito");
        assert_eq!(h.range(8, 8).unwrap(), Vec::<f32>::new());
    }
}
