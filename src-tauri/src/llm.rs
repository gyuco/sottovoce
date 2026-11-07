//! Costruzione del client LLM dalle impostazioni correnti.

use std::time::Duration;

use anyhow::{Result, bail};
use sv_llm::OpenAiCompatible;

use crate::secrets;
use crate::settings::{LlmProvider, Settings};

/// Client per il provider configurato. Legge la chiave dal Keychain (può
/// bloccare: va chiamata fuori dal thread principale). `require_key` = errore
/// se il provider la prevede ma non è salvata (per Custom resta opzionale).
pub fn client(settings: &Settings, require_key: bool) -> Result<OpenAiCompatible> {
    let provider = settings.llm_provider;
    let base_url = provider.base_url(&settings.llm_base_url)?;
    let key = match provider.key_account() {
        Some(account) => secrets::get_api_key(account)?,
        None => None,
    };
    let needs_key = matches!(provider, LlmProvider::Opencode | LlmProvider::OpencodeGo);
    if require_key && key.is_none() && needs_key {
        bail!("API key OpenCode mancante: impostala nelle Impostazioni");
    }
    // Largo per il caricamento a freddo dei modelli Ollama (anche 30 s): le
    // chiamate in tempo reale passano un timeout per richiesta più corto.
    let mut client = OpenAiCompatible::new(base_url)
        .with_api_key(key)
        .with_timeout(Duration::from_secs(60));
    if provider == LlmProvider::OpencodeGo {
        // Go rifiuta le richieste senza un id di sessione stabile per
        // conversazione (routing e prompt caching): uno per client.
        client = client.with_header("x-opencode-session", &session_id());
    }
    Ok(client)
}

/// Id univoco abbastanza per una sessione: pid + istante + contatore.
fn session_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "sottovoce-{:x}-{nanos:x}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// Messaggio d'errore per l'utente, con un suggerimento quando utile.
pub fn describe(provider: LlmProvider, err: &sv_llm::LlmError) -> String {
    match (provider, err) {
        (LlmProvider::Ollama, sv_llm::LlmError::Connect(_)) => {
            "Ollama non risponde su localhost:11434: è in esecuzione?".into()
        }
        (_, sv_llm::LlmError::Http { status: 404, .. }) => {
            format!("{err} (modello o indirizzo inesistente?)")
        }
        _ => err.to_string(),
    }
}
