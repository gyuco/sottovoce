//! Vettori quantizzati in int8 (scala per vettore) per l'indice su disco.

/// Vettore int8 con scala: `valore ≈ data[i] * scale`.
#[derive(Debug, Clone, PartialEq)]
pub struct QuantVec {
    pub scale: f32,
    pub data: Vec<i8>,
}

impl QuantVec {
    /// Quantizza simmetricamente sul massimo in valore assoluto.
    pub fn quantize(v: &[f32]) -> Self {
        let max = v.iter().fold(0f32, |m, x| m.max(x.abs()));
        if max <= f32::EPSILON {
            return Self {
                scale: 0.0,
                data: vec![0; v.len()],
            };
        }
        let scale = max / 127.0;
        let data = v
            .iter()
            .map(|x| (x / scale).round().clamp(-127.0, 127.0) as i8)
            .collect();
        Self { scale, data }
    }

    /// Valori ricostruiti.
    pub fn dequantize(&self) -> Vec<f32> {
        self.data
            .iter()
            .map(|&q| f32::from(q) * self.scale)
            .collect()
    }

    /// Prodotto scalare con un vettore f32 (la query resta in f32). Con
    /// vettori normalizzati è la similarità coseno.
    pub fn dot(&self, query: &[f32]) -> f32 {
        let sum: f32 = self
            .data
            .iter()
            .zip(query)
            .map(|(&q, &x)| f32::from(q) * x)
            .sum();
        sum * self.scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::truncate;

    #[test]
    fn round_trip_is_close() {
        let v: Vec<f32> = (0..512)
            .map(|i| ((i * 37 % 101) as f32 - 50.0) / 50.0)
            .collect();
        let v = truncate(&v, 512);
        let q = QuantVec::quantize(&v);
        let back = q.dequantize();
        let max_err = v
            .iter()
            .zip(&back)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(max_err <= q.scale / 2.0 + 1e-6, "{max_err}");
        assert!((q.dot(&v) - 1.0).abs() < 1e-3, "{}", q.dot(&v));
    }

    #[test]
    fn zero_vector() {
        let q = QuantVec::quantize(&[0.0; 4]);
        assert_eq!(q.data, vec![0; 4]);
        assert_eq!(q.dot(&[1.0; 4]), 0.0);
    }
}
