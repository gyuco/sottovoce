//! Test sul modello vero: saltati se il modello non è scaricato.
//! Cartella: `SV_EMBED_MODEL_DIR`, altrimenti quella dell'app.

use std::path::PathBuf;

use sv_embed::{DIM, Embedder, document_prompt};

fn model_dir() -> Option<PathBuf> {
    let dir = match std::env::var("SV_EMBED_MODEL_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => PathBuf::from(std::env::var("HOME").ok()?).join(
            "Library/Application Support/com.giuseppeconcas.sottovoce/models/embeddinggemma-2-q4",
        ),
    };
    dir.join(sv_embed::MODEL_FILE).exists().then_some(dir)
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[test]
fn ranks_relevant_passages_across_languages() {
    let Some(dir) = model_dir() else {
        eprintln!("modello assente: test saltato");
        return;
    };
    let mut e = Embedder::open(&dir, 2).unwrap();
    let q = e
        .embed_query("quanto costa l'hosting del progetto?")
        .unwrap();
    assert_eq!(q.len(), DIM);
    assert!((dot(&q, &q) - 1.0).abs() < 1e-4);

    let docs = [
        document_prompt(
            None,
            "The AWS quote for the servers is way too high, we need to cut it.",
        ),
        document_prompt(
            None,
            "Let's schedule the next sprint review for Thursday afternoon.",
        ),
        document_prompt(None, "Il preventivo per i server cloud è troppo alto."),
    ];
    let d = e.embed_documents(&docs).unwrap();
    let s: Vec<f32> = d.iter().map(|v| dot(&q, v)).collect();
    assert!(s[0] > s[1] + 0.05, "{s:?}");
    assert!(s[2] > s[1] + 0.05, "{s:?}");
}

#[test]
fn batching_matches_single_runs() {
    let Some(dir) = model_dir() else {
        eprintln!("modello assente: test saltato");
        return;
    };
    let mut e = Embedder::open(&dir, 2).unwrap();
    let docs = [
        document_prompt(Some("Sprint"), "Marco: ok."),
        document_prompt(
            Some("Sprint"),
            "Anna: rivediamo le metriche di latenza della pipeline di deploy prima del rilascio.",
        ),
    ];
    let batch = e.embed_documents(&docs).unwrap();
    for (doc, b) in docs.iter().zip(&batch) {
        let single = e.embed_documents(std::slice::from_ref(doc)).unwrap();
        assert!(dot(b, &single[0]) > 0.999, "{}", dot(b, &single[0]));
    }
}
