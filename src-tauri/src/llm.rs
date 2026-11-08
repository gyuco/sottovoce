//! Costruzione del client LLM dalle impostazioni correnti.

use std::time::Duration;

use std::sync::Arc;

use anyhow::{Result, bail};
use sv_llm::{Anthropic, LlmProvider, ModelInfo, OpenAiCompatible};

use crate::secrets;
use crate::settings::{LlmProvider as Provider, Settings};

/// Client per `provider` (l'indirizzo di Custom viene da `settings`). Legge
/// la chiave dal Keychain (può bloccare: va chiamata fuori dal thread
/// principale). `require_key` = errore se il provider la prevede ma non è
/// salvata (per Custom resta opzionale).
pub fn client(
    settings: &Settings,
    provider: Provider,
    require_key: bool,
) -> Result<Arc<dyn LlmProvider>> {
    let base_url = provider.base_url(&settings.llm_base_url)?;
    let key = match provider.key_account() {
        Some(account) => secrets::get_api_key(account)?,
        None => None,
    };
    let needs_key = !matches!(provider, Provider::Ollama | Provider::Custom);
    if key.is_none() && needs_key && (require_key || provider == Provider::Anthropic) {
        bail!("API key mancante per il modello linguistico: impostala nelle Impostazioni");
    }
    // Largo per il caricamento a freddo dei modelli Ollama (anche 30 s): le
    // chiamate in tempo reale passano un timeout per richiesta più corto.
    let timeout = Duration::from_secs(60);
    if provider == Provider::Anthropic {
        let key = key.unwrap_or_default();
        return Ok(Arc::new(Anthropic::new(key).with_timeout(timeout)));
    }
    let mut client = OpenAiCompatible::new(base_url)
        .with_api_key(key)
        .with_timeout(timeout);
    if provider == Provider::OpencodeGo {
        // Go rifiuta le richieste senza un id di sessione stabile per
        // conversazione (routing e prompt caching): uno per client.
        client = client.with_header("x-opencode-session", &session_id());
    }
    Ok(Arc::new(client))
}

/// Segna come non usabili i modelli che il provider non serve in chat.
/// OpenAI elenca anche embedding, audio, immagini e modelli solo
/// `/v1/responses`, senza dire quali: si filtra per nome.
pub fn mark_usable(provider: Provider, models: &mut [ModelInfo]) {
    if provider == Provider::Openai {
        for m in models {
            m.usable = openai_chat_model(&m.id);
        }
    }
}

fn openai_chat_model(id: &str) -> bool {
    const FAMILIES: [&str; 5] = ["gpt-", "chatgpt-", "o1", "o3", "o4"];
    const EXCLUDED: [&str; 10] = [
        "audio",
        "realtime",
        "transcribe",
        "tts",
        "image",
        "search",
        "embedding",
        "instruct",
        "codex",
        "-pro",
    ];
    FAMILIES.iter().any(|f| id.starts_with(f)) && !EXCLUDED.iter().any(|x| id.contains(x))
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
pub fn describe(provider: Provider, err: &sv_llm::LlmError) -> String {
    match (provider, err) {
        (Provider::Ollama, sv_llm::LlmError::Connect(_)) => {
            "Ollama non risponde su localhost:11434: è in esecuzione?".into()
        }
        (_, sv_llm::LlmError::Http { status: 404, .. }) => {
            format!("{err} (modello o indirizzo inesistente?)")
        }
        _ => err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::openai_chat_model;

    #[test]
    fn openai_chat_models_by_name() {
        for id in [
            "gpt-5.5",
            "gpt-5.4-mini",
            "gpt-4o",
            "o4-mini",
            "chatgpt-4o-latest",
        ] {
            assert!(openai_chat_model(id), "{id}");
        }
        for id in [
            "text-embedding-3-small",
            "gpt-4o-realtime-preview",
            "gpt-4o-mini-tts",
            "gpt-image-1",
            "gpt-5-codex",
            "o3-pro",
            "whisper-1",
            "dall-e-3",
            "gpt-4o-search-preview",
        ] {
            assert!(!openai_chat_model(id), "{id}");
        }
    }
}
