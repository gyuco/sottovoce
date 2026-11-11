//! Fusione della ricerca semantica (passaggi) con quella per parole
//! (meeting) tramite Reciprocal Rank Fusion.

use std::collections::HashMap;

use crate::index::PassageHit;

/// Costante di RRF: smorza il peso delle prime posizioni (valore standard).
pub const RRF_K: f64 = 60.0;

/// Meeting trovato, con i suoi passaggi migliori.
#[derive(Debug, Clone, PartialEq)]
pub struct MeetingHit {
    pub meeting_id: i64,
    pub score: f64,
    /// Trovato anche dalla ricerca per parole.
    pub lexical: bool,
    /// Passaggi semantici, dal più simile.
    pub passages: Vec<PassageHit>,
}

/// Unisce i passaggi semantici (già ordinati per punteggio) e i meeting della
/// ricerca per parole (già ordinati). Il rango semantico di un meeting è
/// quello del suo passaggio migliore; il punteggio è `Σ 1 / (K + rango)`.
pub fn fuse(semantic: &[PassageHit], lexical: &[i64], per_meeting: usize) -> Vec<MeetingHit> {
    let mut order: Vec<i64> = Vec::new();
    let mut hits: HashMap<i64, MeetingHit> = HashMap::new();
    for p in semantic {
        let hit = hits.entry(p.meeting_id).or_insert_with(|| {
            order.push(p.meeting_id);
            let rank = order.len() as f64;
            MeetingHit {
                meeting_id: p.meeting_id,
                score: 1.0 / (RRF_K + rank),
                lexical: false,
                passages: Vec::new(),
            }
        });
        if hit.passages.len() < per_meeting {
            hit.passages.push(p.clone());
        }
    }
    for (i, &id) in lexical.iter().enumerate() {
        let hit = hits.entry(id).or_insert_with(|| MeetingHit {
            meeting_id: id,
            score: 0.0,
            lexical: false,
            passages: Vec::new(),
        });
        hit.lexical = true;
        hit.score += 1.0 / (RRF_K + (i + 1) as f64);
    }
    let mut out: Vec<MeetingHit> = hits.into_values().collect();
    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b.meeting_id.cmp(&a.meeting_id))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ChunkRef;

    fn p(meeting_id: i64, score: f32) -> PassageHit {
        PassageHit {
            meeting_id,
            source: ChunkRef::Overview,
            start_ms: 0,
            score,
        }
    }

    #[test]
    fn both_sources_beat_one() {
        let semantic = [p(1, 0.8), p(2, 0.7), p(1, 0.6), p(1, 0.5)];
        let out = fuse(&semantic, &[2, 3], 2);
        let ids: Vec<i64> = out.iter().map(|h| h.meeting_id).collect();
        // 2 è secondo in semantica e primo per parole: vince.
        assert_eq!(ids, vec![2, 1, 3]);
        assert!(out[0].lexical && !out[1].lexical && out[2].lexical);
        // Al massimo 2 passaggi per meeting, nell'ordine dato.
        assert_eq!(out[1].passages.len(), 2);
        assert_eq!(out[1].passages[0].score, 0.8);
        assert!(out[2].passages.is_empty());
    }

    #[test]
    fn lexical_only_keeps_its_order() {
        let ids: Vec<i64> = fuse(&[], &[5, 9, 7], 3)
            .iter()
            .map(|h| h.meeting_id)
            .collect();
        assert_eq!(ids, vec![5, 9, 7]);
    }
}
