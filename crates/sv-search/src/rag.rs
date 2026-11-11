//! Recupero per le domande sullo storico (RAG).
//!
//! - [`query_terms`] e [`lexical_hits`]: ricerca per parole a livello di
//!   blocco, pesata con l'IDF dei termini (le domande sono frasi intere: la
//!   ricerca "tutte le parole" dello storico non troverebbe nulla).
//! - [`fuse_passages`]: Reciprocal Rank Fusion tra passaggi semantici e per
//!   parole, con un limite per meeting.
//! - [`build_context`]: blocchi numerati `[n]` per il modello, entro un budget
//!   di caratteri; si scartano prima i passaggi con punteggio più basso.
//! - [`citations`] / [`strip_citations`]: lettura delle citazioni `[n]`
//!   nella risposta.

use std::collections::{HashMap, HashSet};

use sv_store::MeetingDetails;

use crate::chunk::{self, Chunk, ChunkRef};
use crate::fuse::RRF_K;
use crate::index::PassageHit;

/// Passaggi massimi nel contesto (riepiloghi esclusi).
pub const MAX_PASSAGES: usize = 15;
/// Passaggi massimi per meeting.
pub const PER_MEETING: usize = 4;
/// Budget del contesto in caratteri.
pub const CONTEXT_BUDGET: usize = 6_000;
/// Caratteri massimi del testo di un singolo blocco.
const MAX_ITEM_CHARS: usize = 900;
/// Il riepilogo di un meeting trovato entra appena sotto il suo passaggio
/// migliore: è compatto e spesso contiene la decisione cercata.
const SUMMARY_WEIGHT: f64 = 0.95;

/// Parole senza contenuto (it/en/es) escluse dalla ricerca per parole.
#[rustfmt::skip]
const STOPWORDS: &[&str] = &[
    // it
    "il", "lo", "la", "le", "gli", "un", "uno", "una", "di", "da", "del", "della", "dei", "delle",
    "dello", "degli", "al", "alla", "ai", "alle", "allo", "agli", "nel", "nella", "nei", "nelle",
    "sul", "sulla", "sui", "sulle", "con", "per", "tra", "fra", "che", "chi", "cosa", "come",
    "quando", "dove", "quale", "quali", "quanto", "quanti", "perche", "non", "piu", "ma", "ed",
    "se", "si", "mi", "ti", "ci", "vi", "ne", "io", "tu", "lui", "lei", "noi", "voi", "loro",
    "sono", "era", "erano", "stato", "stata", "essere", "avere", "abbiamo", "avevamo", "avete",
    "hanno", "aveva", "ha", "ho", "hai", "fatto", "fare", "detto", "doveva", "dovevamo", "deciso",
    "questo", "questa", "quello", "quella", "anche", "poi", "gia", "ancora", "molto", "tutto",
    "tutti", "nostro", "nostra", "meeting", "riunione", "call",
    // en
    "the", "an", "and", "or", "of", "to", "in", "on", "at", "for", "with", "from", "by", "about",
    "is", "are", "was", "were", "be", "been", "have", "has", "had", "do", "did", "does", "what",
    "who", "whom", "which", "when", "where", "why", "how", "we", "you", "he", "she", "it", "they",
    "our", "my", "your", "their", "this", "that", "these", "those", "not", "but", "if", "so", "did",
    "decide", "decided", "said", "say", "supposed", "should", "would", "could", "will",
    // es
    "el", "los", "las", "unos", "unas", "de", "del", "en", "con", "por", "para", "que", "quien",
    "como", "cuando", "donde", "cual", "es", "son", "fue", "eran", "hemos", "habiamos", "tenia",
    "nosotros", "ellos", "su", "sus", "nuestro", "reunion", "decidimos",
];

/// Termini di una domanda: forma normalizzata, senza parole vuote né
/// duplicati, nell'ordine in cui compaiono.
pub fn query_terms(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    sv_store::normalize(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2 && !STOPWORDS.contains(t))
        .filter(|t| seen.insert((*t).to_owned()))
        .map(str::to_owned)
        .collect()
}

/// Radice grossolana: "rinnovo" trova anche "rinnovare", "preventivo"
/// anche "preventivi". Le parole corte restano intere (prefisso).
fn stem(term: &str) -> &str {
    let n = term.chars().count();
    if n < 6 {
        return term;
    }
    let keep = (n - 2).max(5);
    term.char_indices()
        .nth(keep)
        .map_or(term, |(i, _)| &term[..i])
}

fn tokens(text: &str) -> Vec<String> {
    sv_store::normalize(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Passaggi trovati per parole tra i blocchi dati (meeting, blocchi). Il
/// punteggio di un blocco è la somma dell'IDF dei termini che contiene; i
/// blocchi senza termini restano fuori. Al massimo `limit`, dal migliore.
pub fn lexical_hits(
    meetings: &[(i64, Vec<Chunk>)],
    terms: &[String],
    limit: usize,
) -> Vec<PassageHit> {
    if terms.is_empty() {
        return Vec::new();
    }
    let stems: Vec<&str> = terms.iter().map(|t| stem(t)).collect();
    // Per ogni blocco: quali termini contiene.
    let mut matched: Vec<(i64, &Chunk, Vec<bool>)> = Vec::new();
    let mut df = vec![0usize; terms.len()];
    let mut total = 0usize;
    for (meeting_id, chunks) in meetings {
        for chunk in chunks {
            total += 1;
            let toks = tokens(&chunk.text);
            let hits: Vec<bool> = stems
                .iter()
                .map(|s| toks.iter().any(|t| t.starts_with(s)))
                .collect();
            if hits.iter().any(|&h| h) {
                for (i, &h) in hits.iter().enumerate() {
                    df[i] += usize::from(h);
                }
                matched.push((*meeting_id, chunk, hits));
            }
        }
    }
    let idf: Vec<f32> = df
        .iter()
        .map(|&d| (1.0 + total as f32 / d.max(1) as f32).ln())
        .collect();
    let mut out: Vec<PassageHit> = matched
        .into_iter()
        .map(|(meeting_id, chunk, hits)| PassageHit {
            meeting_id,
            source: chunk.source,
            start_ms: chunk.start_ms,
            score: hits
                .iter()
                .zip(&idf)
                .filter(|(h, _)| **h)
                .map(|(_, w)| w)
                .sum(),
        })
        .collect();
    // Ordinamento stabile: a parità vale l'ordine dei meeting dato.
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.truncate(limit);
    out
}

/// Passaggio scelto per il contesto, con il punteggio fuso.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub meeting_id: i64,
    pub source: ChunkRef,
    pub start_ms: u64,
    pub score: f64,
}

/// Reciprocal Rank Fusion per passaggio: un blocco trovato da entrambe le
/// ricerche sale. Al massimo `per_meeting` passaggi per meeting e `limit` in
/// tutto, dal migliore.
pub fn fuse_passages(
    semantic: &[PassageHit],
    lexical: &[PassageHit],
    per_meeting: usize,
    limit: usize,
) -> Vec<Candidate> {
    let mut order: Vec<(i64, ChunkRef)> = Vec::new();
    let mut by_key: HashMap<(i64, ChunkRef), Candidate> = HashMap::new();
    for list in [semantic, lexical] {
        for (i, p) in list.iter().enumerate() {
            let key = (p.meeting_id, p.source);
            let c = by_key.entry(key).or_insert_with(|| {
                order.push(key);
                Candidate {
                    meeting_id: p.meeting_id,
                    source: p.source,
                    start_ms: p.start_ms,
                    score: 0.0,
                }
            });
            c.score += 1.0 / (RRF_K + (i + 1) as f64);
        }
    }
    let mut all: Vec<Candidate> = order.iter().filter_map(|k| by_key.remove(k)).collect();
    all.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut per: HashMap<i64, usize> = HashMap::new();
    all.retain(|c| {
        let n = per.entry(c.meeting_id).or_default();
        *n += 1;
        *n <= per_meeting
    });
    all.truncate(limit);
    all
}

/// Blocco del contesto, numerato come lo vede il modello.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    /// Numero della citazione `[n]`, da 1.
    pub n: u32,
    pub meeting_id: i64,
    pub meeting_name: String,
    pub started_at_ms: i64,
    pub source: ChunkRef,
    pub start_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Context {
    /// Righe `[n] <meeting> · <data> · <mm:ss> · <testo>`.
    pub text: String,
    pub sources: Vec<Source>,
}

struct Item<'a> {
    details: &'a MeetingDetails,
    chunk: Chunk,
    score: f64,
}

/// Ordine dentro un meeting: prima riepilogo e action item, poi il resto
/// nel tempo.
fn kind_rank(source: ChunkRef) -> u8 {
    match source {
        ChunkRef::Overview => 0,
        ChunkRef::ActionItems => 1,
        ChunkRef::KeyPoints { .. } => 2,
        ChunkRef::Block { .. } => 3,
        ChunkRef::Transcript { .. } => 4,
    }
}

/// Costruisce il contesto: i passaggi `candidates` (dal migliore) più
/// panoramica e action item dei meeting trovati. Si prendono i blocchi dal
/// punteggio più alto finché stanno nel `budget`; poi si raggruppano per
/// meeting (dal più recente) e si numerano. `utc_offset_min` = minuti da
/// aggiungere a UTC per l'ora locale (date nel contesto).
pub fn build_context(
    candidates: &[Candidate],
    meetings: &HashMap<i64, MeetingDetails>,
    budget: usize,
    utc_offset_min: i32,
) -> Context {
    let mut chunks_of: HashMap<i64, Vec<Chunk>> = HashMap::new();
    let mut items: Vec<Item<'_>> = Vec::new();
    let mut seen: HashSet<(i64, ChunkRef)> = HashSet::new();
    let mut best: Vec<(i64, f64)> = Vec::new();
    for c in candidates {
        let Some(details) = meetings.get(&c.meeting_id) else {
            continue;
        };
        let chunks = chunks_of
            .entry(c.meeting_id)
            .or_insert_with(|| chunk::chunks(details));
        // Il meeting è cambiato dopo l'indicizzazione: il blocco non c'è più.
        let Some(chunk) = chunks.iter().find(|ch| ch.source == c.source) else {
            continue;
        };
        if !seen.insert((c.meeting_id, c.source)) {
            continue;
        }
        if !best.iter().any(|(id, _)| *id == c.meeting_id) {
            best.push((c.meeting_id, c.score));
        }
        items.push(Item {
            details,
            chunk: chunk.clone(),
            score: c.score,
        });
    }
    for (meeting_id, score) in best {
        let details = &meetings[&meeting_id];
        for chunk in &chunks_of[&meeting_id] {
            if matches!(chunk.source, ChunkRef::Overview | ChunkRef::ActionItems)
                && seen.insert((meeting_id, chunk.source))
            {
                items.push(Item {
                    details,
                    chunk: chunk.clone(),
                    score: score * SUMMARY_WEIGHT,
                });
            }
        }
    }

    // Budget: dal punteggio più alto; chi non sta resta fuori.
    items.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut used = 0;
    let mut kept: Vec<Item<'_>> = Vec::new();
    for item in items {
        // Il numero vero si assegna dopo: "[99] " ha la lunghezza massima
        // realistica.
        let len = line(99, &item, utc_offset_min).chars().count() + 1;
        if used + len > budget {
            continue;
        }
        used += len;
        kept.push(item);
    }

    kept.sort_by(|a, b| {
        b.details
            .meeting
            .started_at_ms
            .cmp(&a.details.meeting.started_at_ms)
            .then_with(|| b.details.meeting.id.cmp(&a.details.meeting.id))
            .then_with(|| kind_rank(a.chunk.source).cmp(&kind_rank(b.chunk.source)))
            .then_with(|| a.chunk.start_ms.cmp(&b.chunk.start_ms))
    });
    let mut text = String::new();
    let mut sources = Vec::with_capacity(kept.len());
    for (i, item) in kept.iter().enumerate() {
        let n = u32::try_from(i + 1).unwrap_or(u32::MAX);
        text.push_str(&line(n, item, utc_offset_min));
        text.push('\n');
        sources.push(Source {
            n,
            meeting_id: item.details.meeting.id,
            meeting_name: item.details.meeting.name.trim().to_owned(),
            started_at_ms: item.details.meeting.started_at_ms,
            source: item.chunk.source,
            start_ms: item.chunk.start_ms,
            text: item.chunk.text.clone(),
        });
    }
    Context { text, sources }
}

fn line(n: u32, item: &Item<'_>, utc_offset_min: i32) -> String {
    let name = one_line(item.details.meeting.name.trim());
    let name = if name.is_empty() {
        "Untitled meeting".to_owned()
    } else {
        name
    };
    let date = date(item.details.meeting.started_at_ms, utc_offset_min);
    let at = match item.chunk.source {
        ChunkRef::Transcript { .. } => clock(item.chunk.start_ms),
        ChunkRef::Block { .. } => format!("{} notes", clock(item.chunk.start_ms)),
        ChunkRef::Overview => "summary".to_owned(),
        ChunkRef::KeyPoints { .. } => "key points".to_owned(),
        ChunkRef::ActionItems => "action items".to_owned(),
    };
    let body = clip(
        &item
            .chunk
            .text
            .lines()
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(" | "),
        MAX_ITEM_CHARS,
    );
    format!("[{n}] {name} · {date} · {at} · {body}")
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

/// `mm:ss`, oppure `h:mm:ss` oltre l'ora.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// Data `AAAA-MM-GG` nell'ora locale data dallo scarto in minuti.
pub fn date(epoch_ms: i64, utc_offset_min: i32) -> String {
    let local = epoch_ms + i64::from(utc_offset_min) * 60_000;
    let days = local.div_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Giorni dal 1970-01-01 → (anno, mese, giorno), calendario gregoriano
/// (algoritmo di Howard Hinnant).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Citazioni `[n]` / `[n, m]` in un testo, ognuna una volta, nell'ordine.
pub fn citations(text: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for (_, nums) in citation_spans(text) {
        for n in nums {
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

/// Testo senza citazioni (per la cronologia: i numeri cambiano a ogni domanda).
pub fn strip_citations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (range, _) in citation_spans(text) {
        out.push_str(text[last..range.start].trim_end_matches(' '));
        last = range.end;
    }
    out.push_str(&text[last..]);
    out
}

/// Posizioni delle citazioni valide: `[` cifre (separate da virgole) `]`.
fn citation_spans(text: &str) -> Vec<(std::ops::Range<usize>, Vec<u32>)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'[' {
            i += 1;
            continue;
        }
        let Some(close) = text[i + 1..].find(']').map(|j| i + 1 + j) else {
            break;
        };
        let inner = &text[i + 1..close];
        let nums: Option<Vec<u32>> = inner
            .split(',')
            .map(|p| {
                let p = p.trim();
                if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                    None
                } else {
                    p.parse().ok()
                }
            })
            .collect();
        match nums {
            Some(nums) if !nums.is_empty() && nums.iter().all(|&n| n > 0) => {
                out.push((i..close + 1, nums));
                i = close + 1;
            }
            _ => i += 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use sv_store::{ActionItem, Summary};

    use super::*;
    use crate::chunk::tests::{details, segment};

    fn meeting(id: i64, name: &str, started: i64, lines: &[(&str, &str)]) -> MeetingDetails {
        let segs = lines
            .iter()
            .enumerate()
            .map(|(i, (ch, text))| {
                let speaker = if *ch == "them" { Some(0) } else { None };
                segment(id * 100 + i as i64, ch, speaker, i as u64 * 10_000, text)
            })
            .collect();
        let mut d = details(segs);
        d.meeting.id = id;
        d.meeting.name = name.into();
        d.meeting.started_at_ms = started;
        d
    }

    fn hit(meeting_id: i64, first: i64, score: f32) -> PassageHit {
        PassageHit {
            meeting_id,
            source: ChunkRef::Transcript { first, last: first },
            start_ms: 0,
            score,
        }
    }

    #[test]
    fn terms_drop_stopwords_and_fold_accents() {
        assert_eq!(
            query_terms("Cosa avevamo deciso con ACME sul rinnovo? Perché la città?"),
            vec!["acme", "rinnovo", "citta"]
        );
        assert_eq!(
            query_terms("Who should send the Hetzner quote?"),
            vec!["send", "hetzner", "quote"]
        );
    }

    #[test]
    fn lexical_prefers_rare_terms_and_matches_stems() {
        let a = meeting(
            1,
            "A",
            0,
            &[("me", "Il rinnovare del contratto ACME è a marzo")],
        );
        let b = meeting(
            2,
            "B",
            0,
            &[("me", "Contratto Hetzner firmato, contratto chiuso")],
        );
        let list = vec![(1, chunk::chunks(&a)), (2, chunk::chunks(&b))];
        let hits = lexical_hits(&list, &query_terms("rinnovo contratto acme"), 10);
        assert_eq!(hits.len(), 2);
        // "acme" e "rinnov…" sono solo in A: vince A.
        assert_eq!(hits[0].meeting_id, 1);
        assert!(hits[0].score > hits[1].score);
        assert!(lexical_hits(&list, &query_terms("zebra"), 10).is_empty());
        assert!(lexical_hits(&list, &[], 10).is_empty());
    }

    #[test]
    fn fusion_rewards_both_lists_and_caps_per_meeting() {
        let semantic = [
            hit(1, 1, 0.8),
            hit(1, 2, 0.7),
            hit(1, 3, 0.6),
            hit(2, 9, 0.5),
        ];
        let lexical = [hit(2, 9, 3.0), hit(1, 3, 1.0)];
        let out = fuse_passages(&semantic, &lexical, 2, 10);
        let keys: Vec<(i64, ChunkRef)> = out.iter().map(|c| (c.meeting_id, c.source)).collect();
        // (2,9) e (1,3) sono in entrambe le liste e salgono; il meeting 1
        // tiene solo 2 passaggi.
        assert_eq!(keys[0], (2, ChunkRef::Transcript { first: 9, last: 9 }));
        assert_eq!(keys[1], (1, ChunkRef::Transcript { first: 3, last: 3 }));
        assert_eq!(out.iter().filter(|c| c.meeting_id == 1).count(), 2);
        assert_eq!(out.len(), 3);
        assert_eq!(fuse_passages(&semantic, &lexical, 4, 2).len(), 2);
    }

    #[test]
    fn context_numbers_groups_and_adds_summaries() {
        let mut old = meeting(
            1,
            "Kickoff",
            0,
            &[("me", "Prima frase"), ("them", "Budget 10k")],
        );
        old.summary = Some(Summary {
            meeting_id: 1,
            language: "it".into(),
            overview: "Kickoff del progetto".into(),
            key_points: Vec::new(),
            action_items: vec![ActionItem {
                text: "Luca manda il preventivo".into(),
                done: false,
            }],
            created_at_ms: 0,
        });
        let new = meeting(
            2,
            "Rinnovo\nACME",
            86_400_000 * 30,
            &[("them", "Rinnoviamo a marzo")],
        );
        let chunks_old = chunk::chunks(&old);
        let chunks_new = chunk::chunks(&new);
        let candidates = vec![
            Candidate {
                meeting_id: 1,
                source: chunks_old[0].source,
                start_ms: 0,
                score: 0.5,
            },
            Candidate {
                meeting_id: 2,
                source: chunks_new[0].source,
                start_ms: 0,
                score: 0.4,
            },
            // Meeting sparito: ignorato.
            Candidate {
                meeting_id: 9,
                source: ChunkRef::Overview,
                start_ms: 0,
                score: 0.9,
            },
        ];
        let meetings = HashMap::from([(1, old), (2, new)]);
        let ctx = build_context(&candidates, &meetings, CONTEXT_BUDGET, 60);
        let lines: Vec<&str> = ctx.text.lines().collect();
        // Più recente prima; nel meeting 1 riepilogo, action item, frasi.
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines[0],
            "[1] Rinnovo ACME · 1970-01-31 · 00:00 · Anna: Rinnoviamo a marzo"
        );
        assert_eq!(
            lines[1],
            "[2] Kickoff · 1970-01-01 · summary · Kickoff del progetto"
        );
        assert_eq!(
            lines[2],
            "[3] Kickoff · 1970-01-01 · action items · Luca manda il preventivo"
        );
        assert_eq!(
            lines[3],
            "[4] Kickoff · 1970-01-01 · 00:00 · Io: Prima frase | Anna: Budget 10k"
        );
        let ns: Vec<u32> = ctx.sources.iter().map(|s| s.n).collect();
        assert_eq!(ns, vec![1, 2, 3, 4]);
        assert_eq!(
            ctx.sources[3].source,
            ChunkRef::Transcript {
                first: 100,
                last: 101
            }
        );
    }

    #[test]
    fn budget_drops_the_lowest_scores_first() {
        let long = "parola ".repeat(30);
        let ms: Vec<MeetingDetails> = (1..=4)
            .map(|id| meeting(id, &format!("M{id}"), id * 1000, &[("me", &long)]))
            .collect();
        let candidates: Vec<Candidate> = ms
            .iter()
            .map(|m| Candidate {
                meeting_id: m.meeting.id,
                source: chunk::chunks(m)[0].source,
                start_ms: 0,
                score: m.meeting.id as f64,
            })
            .collect();
        let meetings: HashMap<i64, MeetingDetails> =
            ms.into_iter().map(|m| (m.meeting.id, m)).collect();
        // Ogni riga è ~240 caratteri: ne stanno 2.
        let ctx = build_context(&candidates, &meetings, 500, 0);
        let kept: Vec<i64> = ctx.sources.iter().map(|s| s.meeting_id).collect();
        assert_eq!(kept, vec![4, 3], "restano i punteggi più alti");
        assert!(ctx.text.chars().count() <= 500);
        assert!(
            build_context(&candidates, &meetings, 10, 0)
                .sources
                .is_empty()
        );
    }

    #[test]
    fn without_the_model_words_alone_build_the_context() {
        let d = meeting(
            1,
            "Infra",
            0,
            &[("them", "Mando io il preventivo Hetzner giovedì")],
        );
        let list = vec![(1, chunk::chunks(&d))];
        let lexical = lexical_hits(&list, &query_terms("Chi manda il preventivo Hetzner?"), 40);
        let candidates = fuse_passages(&[], &lexical, PER_MEETING, MAX_PASSAGES);
        let ctx = build_context(&candidates, &HashMap::from([(1, d)]), CONTEXT_BUDGET, 0);
        assert_eq!(ctx.sources.len(), 1);
        assert!(ctx.text.starts_with("[1] Infra · "));
        assert!(ctx.text.contains("preventivo Hetzner"));
    }

    #[test]
    fn dates_and_clock() {
        assert_eq!(date(0, 0), "1970-01-01");
        assert_eq!(date(0, -60), "1969-12-31");
        // 2026-11-09T23:30Z → in Italia (UTC+1) è già il 10.
        assert_eq!(date(1_794_267_000_000, 0), "2026-11-09");
        assert_eq!(date(1_794_267_000_000, 60), "2026-11-10");
        assert_eq!(clock(75_000), "01:15");
        assert_eq!(clock(3_725_000), "1:02:05");
    }

    #[test]
    fn citations_are_parsed_and_stripped() {
        let text = "Lo manda Luca [2]. Scadenza marzo [1, 3][2]; vedi [x] e [ ] e [0] e [4";
        assert_eq!(citations(text), vec![2, 1, 3]);
        assert_eq!(
            strip_citations(text),
            "Lo manda Luca. Scadenza marzo; vedi [x] e [ ] e [0] e [4"
        );
        assert!(citations("nessuna").is_empty());
        assert_eq!(citations("[12]"), vec![12]);
    }
}
