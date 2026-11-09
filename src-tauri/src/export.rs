//! Export di un meeting su file Markdown.
//!
//! Il frontend compone il testo (etichette tradotte); qui si sceglie il file con
//! il dialogo di sistema e lo si scrive. Il percorso lo decide sempre l'utente:
//! il frontend non può scrivere dove vuole.

use std::path::PathBuf;

use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use tokio::sync::oneshot;

/// Lunghezza massima del nome file proposto (senza estensione).
const MAX_NAME_CHARS: usize = 80;

/// Chiede dove salvare e scrive `contents`. `None` se l'utente annulla.
pub async fn save_markdown(
    app: &AppHandle,
    file_name: &str,
    contents: &str,
) -> Result<Option<PathBuf>, String> {
    let (tx, rx) = oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(format!("{}.md", sanitize_file_name(file_name)))
        .add_filter("Markdown", &["md"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let mut path = path.into_path().map_err(|e| e.to_string())?;
    if path.extension().is_none() {
        path.set_extension("md");
    }
    tokio::fs::write(&path, contents)
        .await
        .map_err(|e| format!("{}: {e}", path.display()))?;
    tracing::info!(path = %path.display(), bytes = contents.len(), "meeting esportato");
    Ok(Some(path))
}

/// Nome file sicuro: niente separatori o caratteri riservati, spazi compattati.
pub fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let compact = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed: String = compact
        .trim_matches('.')
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        "meeting".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_file_name;

    #[test]
    fn removes_separators_and_reserved_chars() {
        assert_eq!(sanitize_file_name("../a/b:c*?"), "a b c");
        assert_eq!(
            sanitize_file_name("Sprint  review\n2026"),
            "Sprint review 2026"
        );
    }

    #[test]
    fn falls_back_when_empty() {
        assert_eq!(sanitize_file_name(""), "meeting");
        assert_eq!(sanitize_file_name(" .. / "), "meeting");
    }

    #[test]
    fn truncates_long_names() {
        let long = "x".repeat(200);
        assert_eq!(sanitize_file_name(&long).chars().count(), 80);
    }
}
