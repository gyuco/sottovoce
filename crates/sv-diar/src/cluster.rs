//! Raggruppamento delle impronte in un numero noto di parlanti.

use crate::Speaker;
use crate::tracker::{dot, normalized};

const MAX_ITERATIONS: usize = 25;

/// K-means sferico (impronte normalizzate, similarità coseno) in
/// esattamente `k` gruppi, o meno se le impronte sono meno di `k`.
/// Inizializzazione farthest-point (deterministica) e numerazione per ordine
/// di comparsa: il primo che parla è sempre "Speaker 1", così le etichette
/// restano stabili quando si ricalcola dopo ogni frase.
pub fn cluster(vectors: &[&[f32]], k: usize) -> Vec<Speaker> {
    if vectors.is_empty() {
        return Vec::new();
    }
    let data: Vec<Vec<f32>> = vectors
        .iter()
        .map(|v| normalized(v).unwrap_or_else(|| v.to_vec()))
        .collect();
    let k = k.clamp(1, data.len());

    let mut centers = vec![data[0].clone()];
    let mut chosen = vec![false; data.len()];
    chosen[0] = true;
    while centers.len() < k {
        let pick = data
            .iter()
            .enumerate()
            .filter(|(j, _)| !chosen[*j])
            .map(|(j, v)| (j, max_sim(v, &centers)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let Some((j, _)) = pick else { break };
        chosen[j] = true;
        centers.push(data[j].clone());
    }

    let mut labels = vec![usize::MAX; data.len()];
    for _ in 0..MAX_ITERATIONS {
        let next: Vec<usize> = data.iter().map(|v| nearest(v, &centers)).collect();
        if next == labels {
            break;
        }
        labels = next;
        for (c, center) in centers.iter_mut().enumerate() {
            let mut sum = vec![0.0f32; center.len()];
            for (v, _) in data.iter().zip(&labels).filter(|(_, l)| **l == c) {
                sum.iter_mut().zip(v).for_each(|(s, x)| *s += x);
            }
            if let Some(n) = normalized(&sum) {
                *center = n;
            }
        }
    }

    let mut remap: Vec<Option<Speaker>> = vec![None; centers.len()];
    let mut next = 0;
    labels
        .iter()
        .map(|&l| {
            *remap[l].get_or_insert_with(|| {
                next += 1;
                next - 1
            })
        })
        .collect()
}

fn max_sim(v: &[f32], centers: &[Vec<f32>]) -> f32 {
    centers
        .iter()
        .map(|c| dot(v, c))
        .fold(f32::NEG_INFINITY, f32::max)
}

fn nearest(v: &[f32], centers: &[Vec<f32>]) -> usize {
    centers
        .iter()
        .enumerate()
        .map(|(i, c)| (i, dot(v, c)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(0, |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::voice;

    fn run(axes: &[usize], k: usize) -> Vec<Speaker> {
        let vs: Vec<Vec<f32>> = axes
            .iter()
            .enumerate()
            .map(|(i, &a)| voice(a, 0.3, i as u32))
            .collect();
        let refs: Vec<&[f32]> = vs.iter().map(Vec::as_slice).collect();
        cluster(&refs, k)
    }

    #[test]
    fn three_voices_k3_numbered_by_appearance() {
        assert_eq!(run(&[2, 0, 2, 1, 0, 1, 2], 3), [0, 1, 0, 2, 1, 2, 0]);
    }

    #[test]
    fn k_larger_than_data_or_one() {
        assert_eq!(run(&[0, 1], 5), [0, 1]);
        assert_eq!(run(&[0, 1, 2], 1), [0, 0, 0]);
        assert!(cluster(&[], 2).is_empty());
    }
}
