//! Preferenze utente persistite in `settings.json` (config dir dell'app).
//! Niente segreti qui: le API key stanno nel Keychain (vedi `secrets`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sv_stt::Language;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SttProvider {
    #[default]
    Deepgram,
}

impl SttProvider {
    /// Account della API key nel Keychain (vedi `secrets::KNOWN_PROVIDERS`).
    pub fn id(self) -> &'static str {
        match self {
            Self::Deepgram => "deepgram",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub stt_provider: SttProvider,
    pub stt_model: String,
    /// Lingua dell'utente (canale "Io"): monolingua = più accurato.
    pub my_language: Language,
    /// Lingua degli altri partecipanti (canale "Loro").
    pub their_language: Language,
    /// `None` = microfono di default di sistema.
    pub mic_device_id: Option<String>,
    pub system_audio: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            stt_provider: SttProvider::Deepgram,
            stt_model: "nova-3".into(),
            my_language: Language::It,
            their_language: Language::Multi,
            mic_device_id: None,
            system_audio: true,
        }
    }
}

pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsStore {
    /// Carica da `path`; file assente o illeggibile → default (con warning).
    pub fn load(path: PathBuf) -> Self {
        let current = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!(path = %path.display(), "settings non validi, uso i default: {e}");
                Settings::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => {
                tracing::warn!(path = %path.display(), "settings illeggibili, uso i default: {e}");
                Settings::default()
            }
        };
        Self {
            path,
            current: Mutex::new(current),
        }
    }

    pub fn get(&self) -> Settings {
        self.current.lock().expect("settings lock").clone()
    }

    /// Applica un patch parziale (chiavi camelCase di [`Settings`]), valida,
    /// salva su disco e ritorna lo stato aggiornato.
    pub fn update(&self, patch: Value) -> Result<Settings> {
        let Value::Object(patch) = patch else {
            bail!("il patch delle impostazioni deve essere un oggetto");
        };
        let mut guard = self.current.lock().expect("settings lock");
        let mut merged = serde_json::to_value(&*guard)?;
        let obj = merged.as_object_mut().expect("Settings è un oggetto");
        for (k, v) in patch {
            if !obj.contains_key(&k) {
                bail!("impostazione sconosciuta: {k}");
            }
            obj.insert(k, v);
        }
        let next: Settings =
            serde_json::from_value(merged).context("valore di impostazione non valido")?;
        save(&self.path, &next)?;
        *guard = next.clone();
        Ok(next)
    }
}

/// Scrittura atomica: file temporaneo + rename.
fn save(path: &Path, settings: &Settings) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).context("impossibile creare la cartella di config")?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(settings)?)
        .context("impossibile scrivere le impostazioni")?;
    std::fs::rename(&tmp, path).context("impossibile salvare le impostazioni")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sottovoce-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("settings.json")
    }

    #[test]
    fn missing_file_gives_defaults() {
        let store = SettingsStore::load(temp_path("missing"));
        assert_eq!(store.get(), Settings::default());
    }

    #[test]
    fn update_persists_and_reloads() {
        let path = temp_path("persist");
        let store = SettingsStore::load(path.clone());
        let s = store
            .update(json!({"theirLanguage": "it", "micDeviceId": "abc", "systemAudio": false}))
            .unwrap();
        assert_eq!(s.their_language, Language::It);
        assert_eq!(s.mic_device_id.as_deref(), Some("abc"));
        assert_eq!(SettingsStore::load(path.clone()).get(), s);
        // null riporta al microfono di default.
        let s = store.update(json!({"micDeviceId": null})).unwrap();
        assert_eq!(s.mic_device_id, None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn rejects_unknown_key_and_bad_value() {
        let store = SettingsStore::load(temp_path("reject"));
        assert!(store.update(json!({"nope": 1})).is_err());
        assert!(store.update(json!({"myLanguage": "klingon"})).is_err());
        assert!(store.update(json!([1, 2])).is_err());
        assert_eq!(store.get(), Settings::default());
    }

    #[test]
    fn corrupt_file_gives_defaults() {
        let path = temp_path("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(SettingsStore::load(path.clone()).get(), Settings::default());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
