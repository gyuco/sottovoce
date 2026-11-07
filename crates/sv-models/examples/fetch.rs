//! Scarica e verifica un modello da terminale, nella stessa cartella dell'app.
//!
//! cargo run -p sv-models --example fetch -- [id] [cartella]
//! Senza argomenti elenca i modelli e il loro stato.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use sv_models::{ModelStore, catalog, find};

/// Cartella usata dall'app su macOS (app data dir + `models`).
fn default_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join("Library/Application Support/com.giuseppeconcas.sottovoce/models")
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let id = args.next();
    let dir = args.next().map(PathBuf::from).unwrap_or_else(default_dir);
    let store = ModelStore::new(&dir);
    println!("cartella: {}", dir.display());

    let Some(id) = id else {
        for m in catalog() {
            println!(
                "{:28} {:>6} MB  {:?}",
                m.id,
                m.size() / 1_000_000,
                store.status(m)
            );
        }
        return;
    };
    let Some(spec) = find(&id) else {
        eprintln!("modello sconosciuto: {id}");
        std::process::exit(2);
    };
    let mut last_pct = u64::MAX;
    let result = store
        .download(
            spec,
            |p| {
                let pct = p.downloaded * 100 / p.total.max(1);
                if pct != last_pct {
                    last_pct = pct;
                    eprint!(
                        "\r{id}: {pct:3}%  ({} / {} MB)",
                        p.downloaded / 1_000_000,
                        p.total / 1_000_000
                    );
                }
            },
            &AtomicBool::new(false),
        )
        .await;
    eprintln!();
    match result {
        Ok(()) => println!("pronto: {}", store.model_dir(spec).display()),
        Err(e) => {
            eprintln!("errore: {e}");
            std::process::exit(1);
        }
    }
}
