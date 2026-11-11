//! Embedding di prova da terminale.
//!
//! `cargo run --release -p sv-embed --example embed -- "query" ["documento" ...]`
//!
//! Cartella del modello: `SV_EMBED_MODEL_DIR`, altrimenti quella dell'app.

use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(query) = args.next() else {
        eprintln!("uso: embed \"query\" [\"documento\" ...]");
        std::process::exit(2);
    };
    let docs: Vec<String> = args.collect();
    let dir = model_dir();

    let t = Instant::now();
    let mut embedder = sv_embed::Embedder::open(&dir, 4).expect("modello");
    println!("modello caricato in {:?} da {}", t.elapsed(), dir.display());

    let t = Instant::now();
    let q = embedder.embed_query(&query).expect("query");
    let norm = q.iter().map(|x| x * x).sum::<f32>().sqrt();
    println!(
        "query: {} dimensioni, norma {norm:.4}, {:?}",
        q.len(),
        t.elapsed()
    );

    if docs.is_empty() {
        return;
    }
    let prompts: Vec<String> = docs
        .iter()
        .map(|d| sv_embed::document_prompt(None, d))
        .collect();
    let t = Instant::now();
    let embs = embedder.embed_documents(&prompts).expect("documenti");
    println!("{} documenti in {:?}", docs.len(), t.elapsed());
    for (doc, e) in docs.iter().zip(&embs) {
        let score: f32 = q.iter().zip(e).map(|(a, b)| a * b).sum();
        println!("{score:.4}  {doc}");
    }
}

fn model_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SV_EMBED_MODEL_DIR") {
        return dir.into();
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join("Library/Application Support/com.giuseppeconcas.sottovoce/models/embeddinggemma-2-q4")
}
