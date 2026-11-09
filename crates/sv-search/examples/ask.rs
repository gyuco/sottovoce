//! Recupero per le domande sullo storico su meeting sintetici it/en.
//!
//! `cargo run --release -p sv-search --example ask -- ["domanda" ...]`
//!
//! Indicizza in memoria (cartella temporanea) quattro meeting finti, due in
//! italiano e due in inglese, e per ogni domanda stampa il contesto numerato
//! che riceverebbe il modello. Senza domande usa quelle di prova e controlla
//! che il passaggio atteso sia nel contesto. Modello: `SV_EMBED_MODEL_DIR`,
//! altrimenti quello dell'app. `--no-model` = solo ricerca per parole.

use std::collections::HashMap;
use std::path::PathBuf;

use sv_search::{
    CONTEXT_BUDGET, MAX_PASSAGES, PER_MEETING, SemanticIndex, build_context, chunks, fuse_passages,
    lexical_hits, prepare, query_terms,
};
use sv_store::{ActionItem, Meeting, MeetingDetails, Segment, SpeakerName, Summary};

const DAY: i64 = 86_400_000;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let no_model = args.iter().any(|a| a == "--no-model");
    let questions: Vec<String> = args.into_iter().filter(|a| !a.starts_with("--")).collect();

    let meetings = synthetic();
    let dir = std::env::temp_dir().join(format!("sv-ask-{}", std::process::id()));
    let index = SemanticIndex::new(&dir, "embeddinggemma-2-q4", sv_embed::DIM);
    let mut embedder =
        (!no_model).then(|| sv_embed::Embedder::open(&model_dir(), 4).expect("modello"));
    if let Some(e) = embedder.as_mut() {
        for d in &meetings {
            let prepared = prepare(d);
            let vectors = e.embed_documents(&prepared.prompts).expect("embedding");
            index
                .store(d.meeting.id, &prepared, &vectors)
                .expect("indice");
        }
    }
    let by_id: HashMap<i64, MeetingDetails> =
        meetings.iter().map(|d| (d.meeting.id, d.clone())).collect();
    let all_chunks: Vec<_> = meetings.iter().map(|d| (d.meeting.id, chunks(d))).collect();

    let checks: Vec<(String, Option<&str>)> = if questions.is_empty() {
        vec![
            (
                "Cosa avevamo deciso con ACME sul rinnovo?".into(),
                Some("two-year renewal"),
            ),
            (
                "Chi doveva mandare il preventivo Hetzner?".into(),
                Some("preventivo Hetzner"),
            ),
            (
                "Quando scade il certificato SSL?".into(),
                Some("SSL certificate"),
            ),
            (
                "¿Cuánto presupuesto tiene la campaña?".into(),
                Some("3.000 euro"),
            ),
            ("Quanto costa il nuovo ufficio a Berlino?".into(), None),
        ]
    } else {
        questions.into_iter().map(|q| (q, None)).collect()
    };

    let mut failed = 0;
    for (q, expect) in checks {
        let semantic = embedder
            .as_mut()
            .map(|e| index.search(&e.embed_query(&q).expect("query"), 60, 0.08))
            .unwrap_or_default();
        let lexical = lexical_hits(&all_chunks, &query_terms(&q), 40);
        let candidates = fuse_passages(&semantic, &lexical, PER_MEETING, MAX_PASSAGES);
        let ctx = build_context(&candidates, &by_id, CONTEXT_BUDGET, 60);
        println!("» {q}");
        println!(
            "  semantica: {} · parole: {} · fonti: {}",
            semantic.len(),
            lexical.len(),
            ctx.sources.len()
        );
        for line in ctx.text.lines() {
            let short: String = line.chars().take(150).collect();
            println!("  {short}");
        }
        if let Some(needle) = expect {
            // Il passaggio atteso deve stare tra le prime 5 fonti per punteggio.
            let top: Vec<_> = candidates.iter().take(5).collect();
            let found = top.iter().any(|c| {
                sv_search::passage_text(&by_id[&c.meeting_id], c.source)
                    .is_some_and(|t| t.contains(needle))
            });
            println!(
                "  atteso \"{needle}\" nei primi 5: {}",
                if found { "sì" } else { "NO" }
            );
            failed += usize::from(!found);
        }
        println!();
    }
    let _ = std::fs::remove_dir_all(&dir);
    if failed > 0 {
        eprintln!("{failed} controlli non superati");
        std::process::exit(1);
    }
}

fn model_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SV_EMBED_MODEL_DIR") {
        return dir.into();
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join("Library/Application Support/com.giuseppeconcas.sottovoce/models/embeddinggemma-2-q4")
}

fn meeting(id: i64, name: &str, day: i64, lang: &str, lines: &[(&str, &str)]) -> MeetingDetails {
    let segments = lines
        .iter()
        .enumerate()
        .map(|(i, (who, text))| Segment {
            id: id * 1000 + i as i64,
            meeting_id: id,
            session_id: (id * 1000 + i as i64) as u64,
            channel: if *who == "Io" { "me" } else { "them" }.into(),
            speaker: (*who != "Io").then_some(0),
            text: (*text).into(),
            start_ms: i as u64 * 15_000,
            end_ms: i as u64 * 15_000 + 12_000,
            language: Some(lang.into()),
            confidence: None,
        })
        .collect();
    let other = lines
        .iter()
        .find(|(w, _)| *w != "Io")
        .map_or("Anna", |(w, _)| w);
    MeetingDetails {
        meeting: Meeting {
            id,
            name: name.into(),
            goal: String::new(),
            notes: String::new(),
            started_at_ms: 1_790_000_000_000 + day * DAY,
            ended_at_ms: Some(1_790_000_000_000 + day * DAY + 1_800_000),
            stt_engine: "local".into(),
            stt_model: String::new(),
            my_language: "it".into(),
            their_language: lang.into(),
        },
        segments,
        translations: Vec::new(),
        speakers: vec![SpeakerName {
            speaker: 0,
            name: (*other).into(),
        }],
        assists: Vec::new(),
        blocks: Vec::new(),
        summary: None,
    }
}

fn synthetic() -> Vec<MeetingDetails> {
    let acme = meeting(
        1,
        "ACME quarterly",
        0,
        "en",
        &[
            (
                "John",
                "Thanks for joining, let's go through the open points for ACME.",
            ),
            (
                "Io",
                "Sure. First the contract, it expires at the end of February.",
            ),
            (
                "John",
                "We discussed it internally and we agreed on a two-year renewal starting in March, with a five percent discount.",
            ),
            (
                "Io",
                "Perfect, I'll prepare the paperwork and send the signed copy by Friday.",
            ),
            (
                "John",
                "Also, our SSL certificate on the staging server expires on December 3rd, someone has to renew it.",
            ),
            ("Io", "I'll ask the ops team to handle the certificate."),
        ],
    );
    let mut infra = meeting(
        2,
        "Infrastruttura",
        5,
        "it",
        &[
            (
                "Luca",
                "Allora, per i server nuovi abbiamo guardato Hetzner e OVH.",
            ),
            ("Io", "Hetzner mi sembra più conveniente per lo storage."),
            (
                "Luca",
                "D'accordo, mando io il preventivo Hetzner entro giovedì, così lo approviamo.",
            ),
            ("Io", "Ottimo, poi decidiamo la migrazione a gennaio."),
        ],
    );
    infra.summary = Some(Summary {
        meeting_id: 2,
        language: "it".into(),
        overview: "Scelta del fornitore per i nuovi server: Hetzner preferito a OVH.".into(),
        key_points: Vec::new(),
        action_items: vec![ActionItem {
            text: "Luca: inviare il preventivo Hetzner entro giovedì".into(),
            done: false,
        }],
        created_at_ms: 0,
    });
    let sprint = meeting(
        3,
        "Sprint review",
        8,
        "en",
        &[
            (
                "Maria",
                "The login page bug is fixed, we shipped it on Tuesday.",
            ),
            ("Io", "Good. What about the slow dashboard?"),
            (
                "Maria",
                "We added caching, the dashboard loads in under a second now.",
            ),
        ],
    );
    let marketing = meeting(
        4,
        "Marketing",
        12,
        "it",
        &[
            ("Giulia", "La campagna social parte il primo dicembre."),
            ("Io", "Che budget abbiamo?"),
            (
                "Giulia",
                "Abbiamo 3.000 euro per la campagna, metà su LinkedIn e metà su Instagram.",
            ),
        ],
    );
    vec![acme, infra, sprint, marketing]
}
