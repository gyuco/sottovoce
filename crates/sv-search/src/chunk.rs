//! Divisione di un meeting in blocchi da indicizzare.
//!
//! - Trascrizione: frasi finali consecutive (entrambi i canali, in ordine di
//!   tempo), ognuna con l'etichetta del parlante, fino a circa
//!   [`TARGET_WORDS`] parole o [`MAX_SEGMENTS`] frasi. Il blocco successivo
//!   riparte dall'ultima frase del precedente: una domanda e la sua risposta
//!   non finiscono separate.
//! - Riepilogo: panoramica, un blocco per argomento dei punti chiave, uno per
//!   le action item.
//! - Compressione: un blocco per ogni finestra di 5 minuti (sommario, temi,
//!   momenti importanti).
//!
//! Le traduzioni non si indicizzano: il modello è multilingue e una query
//! trova già il testo originale in un'altra lingua.

use std::collections::HashMap;

use sv_store::MeetingDetails;

/// Parole oltre cui il blocco di trascrizione si chiude. Blocchi piccoli
/// separano meglio i dettagli puntuali (una cifra, un nome) che in un blocco
/// lungo finiscono diluiti; provato su meeting sintetici it/en con 80/6,
/// 40/3 e 25/2 parole/frasi.
pub const TARGET_WORDS: usize = 40;
/// Frasi massime per blocco di trascrizione.
pub const MAX_SEGMENTS: usize = 3;

/// Da dove viene un blocco. I numeri identificano la sorgente nel meeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChunkRef {
    /// Frasi da `first` a `last` (id delle righe `segments`, inclusi).
    Transcript { first: i64, last: i64 },
    /// Blocco di compressione con questo indice.
    Block { index: u32 },
    /// Panoramica del riepilogo.
    Overview,
    /// Argomento `n` dei punti chiave.
    KeyPoints { n: u32 },
    /// Action item del riepilogo.
    ActionItems,
}

impl ChunkRef {
    /// Codifica compatta per il file dell'indice: (tipo, a, b).
    pub(crate) fn encode(self) -> (u8, i64, i64) {
        match self {
            Self::Transcript { first, last } => (0, first, last),
            Self::Block { index } => (1, i64::from(index), 0),
            Self::Overview => (2, 0, 0),
            Self::KeyPoints { n } => (3, i64::from(n), 0),
            Self::ActionItems => (4, 0, 0),
        }
    }

    pub(crate) fn decode(kind: u8, a: i64, b: i64) -> Option<Self> {
        Some(match kind {
            0 => Self::Transcript { first: a, last: b },
            1 => Self::Block {
                index: u32::try_from(a).ok()?,
            },
            2 => Self::Overview,
            3 => Self::KeyPoints {
                n: u32::try_from(a).ok()?,
            },
            4 => Self::ActionItems,
            _ => return None,
        })
    }
}

/// Blocco di testo da indicizzare.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub source: ChunkRef,
    /// ms dall'avvio del meeting (0 per il riepilogo).
    pub start_ms: u64,
    pub text: String,
}

/// Etichette dei parlanti come nei prompt dell'app: "Io", il nome dato al
/// parlante, "Speaker N", oppure "Loro" senza diarization.
fn speaker_label(names: &HashMap<u8, &str>, channel: &str, speaker: Option<u8>) -> String {
    if channel == "me" {
        return "Io".to_owned();
    }
    match speaker {
        Some(n) => names
            .get(&n)
            .map(|s| (*s).to_owned())
            .unwrap_or_else(|| format!("Speaker {}", u16::from(n) + 1)),
        None => "Loro".to_owned(),
    }
}

fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Blocchi di un meeting, in ordine: trascrizione, compressione, riepilogo.
pub fn chunks(details: &MeetingDetails) -> Vec<Chunk> {
    let names: HashMap<u8, &str> = details
        .speakers
        .iter()
        .filter(|s| !s.name.trim().is_empty())
        .map(|s| (s.speaker, s.name.trim()))
        .collect();

    let mut segments: Vec<_> = details
        .segments
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .collect();
    segments.sort_by_key(|s| (s.start_ms, s.id));

    let mut out = Vec::new();
    let mut start = 0;
    while start < segments.len() {
        let mut end = start;
        let mut count = 0;
        while end < segments.len() {
            count += words(&segments[end].text);
            end += 1;
            if count >= TARGET_WORDS || end - start >= MAX_SEGMENTS {
                break;
            }
        }
        let group = &segments[start..end];
        let text = group
            .iter()
            .map(|s| {
                format!(
                    "{}: {}",
                    speaker_label(&names, &s.channel, s.speaker),
                    s.text.trim()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        out.push(Chunk {
            source: ChunkRef::Transcript {
                first: group[0].id,
                last: group[group.len() - 1].id,
            },
            start_ms: group[0].start_ms,
            text,
        });
        if end >= segments.len() {
            break;
        }
        // Una frase di sovrapposizione, ma sempre avanti di almeno una.
        start = if end - start > 1 { end - 1 } else { end };
    }

    for block in &details.blocks {
        let mut parts = vec![block.summary.trim().to_owned()];
        if !block.topics.is_empty() {
            parts.push(block.topics.join(", "));
        }
        parts.extend(block.moments.iter().map(|m| m.text.trim().to_owned()));
        let text = parts
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            out.push(Chunk {
                source: ChunkRef::Block { index: block.index },
                start_ms: block.start_ms,
                text,
            });
        }
    }

    if let Some(summary) = &details.summary {
        if !summary.overview.trim().is_empty() {
            out.push(Chunk {
                source: ChunkRef::Overview,
                start_ms: 0,
                text: summary.overview.trim().to_owned(),
            });
        }
        for (n, kp) in summary.key_points.iter().enumerate() {
            let points: Vec<&str> = kp
                .points
                .iter()
                .map(|p| p.trim())
                .filter(|p| !p.is_empty())
                .collect();
            if points.is_empty() {
                continue;
            }
            out.push(Chunk {
                source: ChunkRef::KeyPoints {
                    n: u32::try_from(n).unwrap_or(u32::MAX),
                },
                start_ms: 0,
                text: format!("{}\n{}", kp.topic.trim(), points.join("\n")),
            });
        }
        let items: Vec<&str> = summary
            .action_items
            .iter()
            .map(|a| a.text.trim())
            .filter(|a| !a.is_empty())
            .collect();
        if !items.is_empty() {
            out.push(Chunk {
                source: ChunkRef::ActionItems,
                start_ms: 0,
                text: items.join("\n"),
            });
        }
    }
    out
}

/// Impronta del contenuto indicizzabile (FNV-1a 64, stabile tra versioni di
/// Rust): se cambia, l'indice del meeting va rifatto.
pub fn fingerprint(title: &str, chunks: &[Chunk]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        // Separatore: "ab"+"c" ≠ "a"+"bc".
        h ^= 0xff;
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    eat(title.as_bytes());
    for c in chunks {
        let (kind, a, b) = c.source.encode();
        eat(&[kind]);
        eat(&a.to_le_bytes());
        eat(&b.to_le_bytes());
        eat(&c.start_ms.to_le_bytes());
        eat(c.text.as_bytes());
    }
    h
}

#[cfg(test)]
pub(crate) mod tests {
    use sv_store::{
        ActionItem, CompressionBlock, ImportantMoment, KeyPoint, Meeting, Segment, SpeakerName,
        Summary,
    };

    use super::*;

    pub(crate) fn segment(
        id: i64,
        channel: &str,
        speaker: Option<u8>,
        start: u64,
        text: &str,
    ) -> Segment {
        Segment {
            id,
            meeting_id: 1,
            session_id: id as u64,
            channel: channel.into(),
            speaker,
            text: text.into(),
            start_ms: start,
            end_ms: start + 1000,
            language: None,
            confidence: None,
        }
    }

    pub(crate) fn details(segments: Vec<Segment>) -> MeetingDetails {
        MeetingDetails {
            meeting: Meeting {
                id: 1,
                name: "Sprint review".into(),
                goal: String::new(),
                notes: String::new(),
                started_at_ms: 0,
                ended_at_ms: Some(1),
                stt_engine: "local".into(),
                stt_model: String::new(),
                my_language: "it".into(),
                their_language: "multi".into(),
            },
            segments,
            translations: Vec::new(),
            speakers: vec![SpeakerName {
                speaker: 0,
                name: "Anna".into(),
            }],
            assists: Vec::new(),
            blocks: Vec::new(),
            summary: None,
        }
    }

    #[test]
    fn transcript_chunks_overlap_by_one_segment() {
        // 6 frasi brevi: blocchi da 3 con una frase in comune.
        let segs = (0..6)
            .map(|i| segment(i + 1, "me", None, (i as u64) * 1000, "ok va bene"))
            .collect();
        let c = chunks(&details(segs));
        let refs: Vec<_> = c.iter().map(|c| c.source).collect();
        assert_eq!(
            refs,
            vec![
                ChunkRef::Transcript { first: 1, last: 3 },
                ChunkRef::Transcript { first: 3, last: 5 },
                ChunkRef::Transcript { first: 5, last: 6 },
            ]
        );
        assert_eq!(c[1].start_ms, 2000);
    }

    #[test]
    fn long_segments_close_the_chunk_by_words() {
        let long = "parola ".repeat(25);
        let segs = (0..4)
            .map(|i| segment(i + 1, "them", Some(0), (i as u64) * 1000, &long))
            .collect();
        let c = chunks(&details(segs));
        // 25 + 25 ≥ 40: blocchi da 2 frasi, sovrapposti di una.
        let refs: Vec<_> = c.iter().map(|c| c.source).collect();
        assert_eq!(
            refs,
            vec![
                ChunkRef::Transcript { first: 1, last: 2 },
                ChunkRef::Transcript { first: 2, last: 3 },
                ChunkRef::Transcript { first: 3, last: 4 },
            ]
        );
    }

    #[test]
    fn a_single_huge_segment_still_advances() {
        let huge = "parola ".repeat(500);
        let segs = vec![
            segment(1, "me", None, 0, &huge),
            segment(2, "me", None, 1000, &huge),
        ];
        let c = chunks(&details(segs));
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn labels_use_names_and_order_by_time() {
        let segs = vec![
            segment(2, "them", Some(0), 2000, "Il preventivo è alto."),
            segment(1, "me", None, 1000, "Quanto costa?"),
            segment(3, "them", Some(1), 3000, "Si può ridurre."),
            segment(4, "them", None, 4000, "  "),
        ];
        let c = chunks(&details(segs));
        assert_eq!(c.len(), 1);
        assert_eq!(
            c[0].text,
            "Io: Quanto costa?\nAnna: Il preventivo è alto.\nSpeaker 2: Si può ridurre."
        );
    }

    #[test]
    fn summary_and_blocks_become_chunks() {
        let mut d = details(vec![segment(1, "me", None, 0, "ciao")]);
        d.blocks.push(CompressionBlock {
            id: 1,
            meeting_id: 1,
            index: 0,
            start_ms: 0,
            end_ms: 300_000,
            summary: "Discussione sui costi".into(),
            topics: vec!["costi".into(), "hosting".into()],
            moments: vec![ImportantMoment {
                time_ms: 1000,
                kind: "decision".into(),
                text: "Si passa a un VPS".into(),
            }],
        });
        d.summary = Some(Summary {
            meeting_id: 1,
            language: "it".into(),
            overview: "Riunione sui costi.".into(),
            key_points: vec![
                KeyPoint {
                    topic: "Hosting".into(),
                    points: vec!["AWS troppo caro".into()],
                },
                KeyPoint {
                    topic: "Vuoto".into(),
                    points: vec![" ".into()],
                },
            ],
            action_items: vec![ActionItem {
                text: "Chiedere preventivo".into(),
                done: false,
            }],
            created_at_ms: 1,
        });
        let c = chunks(&d);
        let refs: Vec<_> = c.iter().map(|c| c.source).collect();
        assert_eq!(
            refs,
            vec![
                ChunkRef::Transcript { first: 1, last: 1 },
                ChunkRef::Block { index: 0 },
                ChunkRef::Overview,
                ChunkRef::KeyPoints { n: 0 },
                ChunkRef::ActionItems,
            ]
        );
        assert_eq!(
            c[1].text,
            "Discussione sui costi\ncosti, hosting\nSi passa a un VPS"
        );
        assert_eq!(c[3].text, "Hosting\nAWS troppo caro");
    }

    #[test]
    fn fingerprint_tracks_content() {
        let d = details(vec![segment(1, "them", Some(0), 0, "ciao")]);
        let a = fingerprint("x", &chunks(&d));
        assert_eq!(a, fingerprint("x", &chunks(&d)));
        assert_ne!(a, fingerprint("y", &chunks(&d)));
        let mut renamed = d.clone();
        renamed.speakers[0].name = "Bea".into();
        assert_ne!(a, fingerprint("x", &chunks(&renamed)));
    }

    #[test]
    fn chunk_ref_round_trip() {
        for r in [
            ChunkRef::Transcript { first: 3, last: 9 },
            ChunkRef::Block { index: 4 },
            ChunkRef::Overview,
            ChunkRef::KeyPoints { n: 2 },
            ChunkRef::ActionItems,
        ] {
            let (k, a, b) = r.encode();
            assert_eq!(ChunkRef::decode(k, a, b), Some(r));
        }
        assert_eq!(ChunkRef::decode(9, 0, 0), None);
    }
}
