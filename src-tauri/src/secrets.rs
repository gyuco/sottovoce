//! API key dei provider nel Keychain di sistema.
//!
//! Le chiavi non tornano mai al frontend: la UI può solo salvarle, sapere se
//! esistono e cancellarle. La lettura è riservata al codice Rust.
//!
//! Leggere il segreto può far comparire il prompt di accesso del Keychain
//! (sempre, nelle build dev con firma ad-hoc, dopo ogni rebuild). Per questo:
//! - [`has_api_key`] cerca solo gli attributi, senza leggere il segreto;
//! - la chiave letta resta in cache in memoria per tutta la vita del processo.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use anyhow::{Context, Result, anyhow, bail};
use keyring::{Entry, Error};

const SERVICE: &str = "sottovoce";

/// Provider per cui si accetta una chiave (account nel Keychain).
/// `smtp` è la password del server di posta (invio del riepilogo);
/// `google-client-secret` e `google-calendar` (refresh token) servono alla
/// sincronizzazione di Google Calendar.
pub const KNOWN_PROVIDERS: &[&str] = &[
    "deepgram",
    "opencode",
    "openai",
    "anthropic",
    "custom",
    "smtp",
    "google-client-secret",
    "google-calendar",
];

static CACHE: LazyLock<Mutex<HashMap<&'static str, String>>> = LazyLock::new(Default::default);

fn known(provider: &str) -> Result<&'static str> {
    KNOWN_PROVIDERS
        .iter()
        .find(|p| **p == provider)
        .copied()
        .ok_or_else(|| anyhow!("provider sconosciuto: {provider}"))
}

fn entry(provider: &str) -> Result<Entry> {
    Entry::new(SERVICE, known(provider)?).context("Keychain non disponibile")
}

fn cache() -> std::sync::MutexGuard<'static, HashMap<&'static str, String>> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_api_key(provider: &str, key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty() {
        bail!("la API key è vuota");
    }
    entry(provider)?
        .set_password(key)
        .context("impossibile salvare la API key nel Keychain")?;
    cache().insert(known(provider)?, key.to_owned());
    Ok(())
}

/// `None` se la chiave non è salvata. Può mostrare il prompt del Keychain
/// (solo la prima volta per processo).
pub fn get_api_key(provider: &str) -> Result<Option<String>> {
    let provider = known(provider)?;
    if let Some(key) = cache().get(provider) {
        return Ok(Some(key.clone()));
    }
    match entry(provider)?.get_password() {
        Ok(key) => {
            cache().insert(provider, key.clone());
            Ok(Some(key))
        }
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(e).context("impossibile leggere la API key dal Keychain"),
    }
}

/// Esistenza della chiave senza leggerne il valore: nessun prompt.
pub fn has_api_key(provider: &str) -> Result<bool> {
    let provider = known(provider)?;
    if cache().contains_key(provider) {
        return Ok(true);
    }
    // `Entry::new` inizializza lo store di default usato da `search`.
    entry(provider)?;
    let spec = HashMap::from([("service", SERVICE), ("user", provider)]);
    let found = keyring_core::Entry::search(&spec).context("Keychain non disponibile")?;
    Ok(!found.is_empty())
}

/// Idempotente: cancellare una chiave assente non è un errore.
pub fn delete_api_key(provider: &str) -> Result<()> {
    let provider = known(provider)?;
    cache().remove(provider);
    match entry(provider)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(e).context("impossibile cancellare la API key dal Keychain"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_provider_and_empty_key() {
        assert!(entry("../evil").is_err());
        assert!(has_api_key("../evil").is_err());
        assert!(set_api_key("deepgram", "   ").is_err());
    }
}

#[cfg(test)]
mod keychain_tests {
    /// Tocca il Keychain reale: `cargo test -p sottovoce -- --ignored has_key`.
    #[test]
    #[ignore]
    fn has_key_does_not_read_secret() {
        println!(
            "deepgram presente: {}",
            super::has_api_key("deepgram").unwrap()
        );
    }
}
