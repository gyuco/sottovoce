//! Indicizza i meeting di una cartella dati e cerca da terminale.
//!
//! `cargo run --release -p sv-search --example search -- <cartella dati> "query" ["query" ...]`
//!
//! La cartella contiene `sottovoce.db` e `transcripts/` (es. una copia di
//! `~/Library/Application Support/com.giuseppeconcas.sottovoce`); l'indice
//! va in `<cartella>/index`. Modello: `SV_EMBED_MODEL_DIR`, altrimenti quello
//! dell'app.

use std::path::PathBuf;
use std::time::Instant;

use sv_search::{SemanticIndex, fuse, passage_text, prepare};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next().map(PathBuf::from) else {
        eprintln!("uso: search <cartella dati> \"query\" ...");
        std::process::exit(2);
    };
    let queries: Vec<String> = args.collect();
    let store = sv_store::Store::open(dir.join("sottovoce.db")).expect("database");
    let index = SemanticIndex::new(dir.join("index"), "embeddinggemma-2-q4", sv_embed::DIM);
    let mut embedder = sv_embed::Embedder::open(&model_dir(), 4).expect("modello");

    let started = Instant::now();
    let mut chunks = 0;
    for m in store.meetings().expect("meeting") {
        let details = store.details(m.id).expect("dettaglio");
        let prepared = prepare(&details);
        if index.is_fresh(m.id, prepared.fingerprint) {
            continue;
        }
        let vectors = embedder
            .embed_documents(&prepared.prompts)
            .expect("embedding");
        index.store(m.id, &prepared, &vectors).expect("indice");
        chunks += prepared.chunks.len();
        println!(
            "indicizzato #{} \"{}\": {} blocchi",
            m.id,
            m.name,
            prepared.chunks.len()
        );
    }
    println!("{chunks} blocchi in {:?}\n", started.elapsed());

    for q in queries {
        let t = Instant::now();
        let v = embedder.embed_query(&q).expect("query");
        let passages = index.search(&v, 40, 0.05);
        let lexical: Vec<i64> = store
            .list_meetings(0, 100, Some(&q))
            .expect("ricerca per parole")
            .meetings
            .iter()
            .map(|m| m.meeting.id)
            .collect();
        let hits = fuse(&passages, &lexical, 2);
        println!("» {q}  ({:?}, parole: {lexical:?})", t.elapsed());
        for h in hits.iter().take(5) {
            let details = store.details(h.meeting_id).expect("dettaglio");
            println!(
                "  #{} {} rrf={:.4}{}",
                h.meeting_id,
                details.meeting.name,
                h.score,
                if h.lexical { " +parole" } else { "" }
            );
            for p in &h.passages {
                let text = passage_text(&details, p.source)
                    .unwrap_or_default()
                    .replace('\n', " / ");
                let short: String = text.chars().take(110).collect();
                println!("     {:.3}  {short}", p.score);
            }
        }
        println!();
    }
}

fn model_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SV_EMBED_MODEL_DIR") {
        return dir.into();
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join("Library/Application Support/com.giuseppeconcas.sottovoce/models/embeddinggemma-2-q4")
}
