//! Preferenze utente persistite in `settings.json` (config dir dell'app).
//! Niente segreti qui: le API key stanno nel Keychain (vedi `secrets`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sv_stt::Language;

/// Motore di trascrizione.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SttEngineKind {
    /// Sul computer (VAD + Parakeet): niente audio a terzi, funziona offline.
    Local,
    /// Deepgram Nova-3 in streaming (cloud, API key).
    #[default]
    Deepgram,
}

/// Account Keychain della API key Deepgram (vedi `secrets::KNOWN_PROVIDERS`).
pub const DEEPGRAM_KEY: &str = "deepgram";

/// Provider LLM. Tutti OpenAI-compatible tranne `Anthropic` (Messages API).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    /// Locale, nessuna chiave: modalità privacy.
    #[default]
    Ollama,
    /// OpenCode Zen, pay-per-use. Solo i modelli serviti su chat completions.
    Opencode,
    /// Abbonamento OpenCode Go (stessa chiave della console di Zen). Pensato
    /// per i coding agent: richiede `x-opencode-session` per conversazione.
    #[serde(rename = "opencode-go")]
    OpencodeGo,
    /// OpenAI, chat completions. Solo i modelli di chat (filtro per nome).
    Openai,
    /// Anthropic, Messages API nativa.
    Anthropic,
    /// Endpoint OpenAI-compatible qualsiasi (`llm_base_url`), chiave opzionale.
    Custom,
}

impl LlmProvider {
    /// Account della API key nel Keychain; `None` = nessuna chiave.
    pub fn key_account(self) -> Option<&'static str> {
        match self {
            Self::Ollama => None,
            Self::Opencode | Self::OpencodeGo => Some("opencode"),
            Self::Openai => Some("openai"),
            Self::Anthropic => Some("anthropic"),
            Self::Custom => Some("custom"),
        }
    }

    /// Base URL dell'API (con la versione). Per `Custom` è quello impostato.
    pub fn base_url(self, custom: &str) -> Result<String> {
        Ok(match self {
            Self::Ollama => "http://localhost:11434/v1".into(),
            Self::Opencode => "https://opencode.ai/inference/openai/v1".into(),
            Self::OpencodeGo => "https://opencode.ai/inference/go/openai/v1".into(),
            Self::Openai => "https://api.openai.com/v1".into(),
            Self::Anthropic => sv_llm::ANTHROPIC_BASE_URL.into(),
            Self::Custom => {
                let url = custom.trim();
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    bail!(
                        "indirizzo dell'endpoint non valido: deve iniziare con http:// o https://"
                    );
                }
                url.to_owned()
            }
        })
    }
}

/// Cifratura della connessione SMTP.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmtpSecurity {
    /// Connessione in chiaro promossa a TLS (porta 587).
    #[default]
    Starttls,
    /// TLS dall'inizio (porta 465).
    Tls,
}

/// Account Keychain della password SMTP (vedi `secrets::KNOWN_PROVIDERS`).
pub const SMTP_KEY: &str = "smtp";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Motore scelto; con `local_only` vale comunque quello locale.
    /// Alias: i file salvati prima del motore locale usavano `sttProvider`.
    #[serde(alias = "sttProvider")]
    pub stt_engine: SttEngineKind,
    /// Modello Deepgram.
    pub stt_model: String,
    /// Modello del catalogo `sv-models` per il motore locale.
    pub local_stt_model: String,
    /// Privacy: STT locale e LLM solo su questa macchina; l'avvio fallisce
    /// se la configurazione richiederebbe un host esterno.
    pub local_only: bool,
    /// Riconoscimento dei parlanti su "Loro" (serve il modello CAM++).
    pub diarization: bool,
    /// Altri partecipanti noti (1–8); 0 = automatico.
    pub their_speakers: u8,
    /// Profili vocali (dato biometrico, spento di default): riconosce nei
    /// meeting successivi i parlanti a cui l'utente ha dato un nome. Solo
    /// in locale, in `app_data_dir/voices/`.
    pub voice_profiles: bool,
    /// Lingua dell'utente (canale "Io"): monolingua = più accurato.
    pub my_language: Language,
    /// Lingua degli altri partecipanti (canale "Loro").
    pub their_language: Language,
    /// `None` = microfono di default di sistema.
    pub mic_device_id: Option<String>,
    pub system_audio: bool,
    pub llm_provider: LlmProvider,
    /// Base URL per `LlmProvider::Custom` (es. `https://api.groq.com/openai/v1`).
    pub llm_base_url: String,
    /// Modello per la traduzione per frase; vuoto = da scegliere.
    pub translation_model: String,
    /// Provider della traduzione; `None` = `llm_provider`.
    pub translation_provider: Option<LlmProvider>,
    /// Traduce da sola ogni frase di "Loro"; spento = solo su richiesta
    /// (pulsante "Traduci" accanto alla frase).
    pub auto_translate: bool,
    /// Con la traduzione automatica traduce anche le frasi di "Io" nella
    /// lingua del meeting (quella di "Loro").
    pub translate_mine: bool,
    /// Modello per il live assist; vuoto = da scegliere.
    pub assist_model: String,
    /// Provider del live assist (e di compressione e riepilogo);
    /// `None` = `llm_provider`.
    pub assist_provider: Option<LlmProvider>,
    /// Genera il riepilogo finale automaticamente allo stop.
    pub auto_summary: bool,
    /// Lingua in cui tradurre; `None` = la lingua dell'utente.
    pub target_language: Option<Language>,
    /// Avvisi durante la call (parli troppo, monologo, nessuna domanda…),
    /// calcolati dalle metriche senza LLM.
    pub nudges: bool,
    /// Suggerimenti del live assist (cosa dire/chiedere). Spenti di default:
    /// si accendono e spengono anche a call in corso.
    pub live_assist: bool,
    /// Cancellazione d'eco sul microfono (AEC3): senza cuffie toglie da "Io"
    /// la voce degli altri risentita dalle casse. Serve l'audio di sistema.
    pub echo_cancellation: bool,
    /// Ricerca per significato nello storico (modello locale di embedding):
    /// indicizza i meeting in background. Senza il modello resta spenta.
    pub semantic_search: bool,
    /// Nome del mittente delle email (riepilogo del meeting).
    pub email_from_name: String,
    /// Indirizzo del mittente; vuoto = invio SMTP non configurato.
    pub email_from: String,
    /// Server SMTP (es. `smtp.gmail.com`); la password è nel Keychain.
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: SmtpSecurity,
    /// Utente SMTP; vuoto = l'indirizzo del mittente.
    pub smtp_user: String,
    /// ID del client OAuth "App desktop" creato dall'utente su Google Cloud
    /// (sincronizzazione di Google Calendar); il client secret è nel Keychain.
    pub google_client_id: String,
    /// Account Google collegato (solo per mostrarlo); vuoto = non collegato.
    pub google_calendar_email: String,
    /// ID applicazione (client) registrata dall'utente su Microsoft Entra ID
    /// (sincronizzazione di Outlook / Microsoft 365); client pubblico, senza
    /// secret.
    pub microsoft_client_id: String,
    /// Tenant di Entra ID: vuoto = `common` (account aziendali e personali);
    /// ID o dominio della directory per un'app registrata su un solo tenant.
    pub microsoft_tenant_id: String,
    /// Account Microsoft collegato (solo per mostrarlo).
    pub microsoft_calendar_email: String,
    /// Setup guidato del primo avvio completato (o saltato). I file salvati
    /// prima del setup guidato non hanno la chiave: valgono come completati.
    pub onboarding_done: bool,
}

/// Modello di un compito LLM con il suo provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskModel {
    pub provider: LlmProvider,
    pub model: String,
}

impl Settings {
    /// Traduzione; `None` se il modello non è scelto.
    pub fn translation_llm(&self) -> Option<TaskModel> {
        task(
            self.translation_provider.unwrap_or(self.llm_provider),
            &self.translation_model,
        )
    }

    /// Live assist; `None` se il modello non è scelto.
    pub fn assist_llm(&self) -> Option<TaskModel> {
        task(
            self.assist_provider.unwrap_or(self.llm_provider),
            &self.assist_model,
        )
    }

    /// Compressione e riepilogo: il modello dell'assist, altrimenti quello
    /// della traduzione.
    pub fn insight_llm(&self) -> Option<TaskModel> {
        self.assist_llm().or_else(|| self.translation_llm())
    }

    /// Lingua delle traduzioni. "Automatica" non è una lingua di arrivo:
    /// in quel caso si ripiega sull'italiano.
    /// Lingua del riepilogo: quella dell'utente; "automatica" → italiano.
    pub fn summary_language(&self) -> Language {
        match self.my_language {
            Language::Multi => Language::It,
            l => l,
        }
    }

    pub fn effective_target_language(&self) -> Language {
        match self.target_language.unwrap_or(self.my_language) {
            Language::Multi => Language::It,
            l => l,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            stt_engine: SttEngineKind::Deepgram,
            stt_model: "nova-3".into(),
            local_stt_model: sv_models::PARAKEET_V3.into(),
            local_only: false,
            diarization: true,
            their_speakers: 0,
            voice_profiles: false,
            my_language: Language::It,
            their_language: Language::Multi,
            mic_device_id: None,
            system_audio: true,
            llm_provider: LlmProvider::Ollama,
            llm_base_url: String::new(),
            translation_model: String::new(),
            translation_provider: None,
            auto_translate: false,
            translate_mine: true,
            assist_model: String::new(),
            assist_provider: None,
            auto_summary: true,
            target_language: None,
            nudges: true,
            live_assist: false,
            echo_cancellation: true,
            semantic_search: true,
            email_from_name: String::new(),
            email_from: String::new(),
            smtp_host: String::new(),
            smtp_port: 587,
            smtp_security: SmtpSecurity::Starttls,
            smtp_user: String::new(),
            google_client_id: String::new(),
            google_calendar_email: String::new(),
            microsoft_client_id: String::new(),
            microsoft_tenant_id: String::new(),
            microsoft_calendar_email: String::new(),
            onboarding_done: false,
        }
    }
}

fn task(provider: LlmProvider, model: &str) -> Option<TaskModel> {
    let model = model.trim();
    (!model.is_empty()).then(|| TaskModel {
        provider,
        model: model.to_owned(),
    })
}

pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsStore {
    /// Carica da `path`; file assente o illeggibile → default (con warning).
    pub fn load(path: PathBuf) -> Self {
        let current = match std::fs::read(&path) {
            Ok(bytes) => parse_saved(&bytes).unwrap_or_else(|e| {
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

/// Settings salvati. Senza `onboardingDone` il file viene da una versione
/// precedente al setup guidato: l'app era già configurata, niente setup.
fn parse_saved(bytes: &[u8]) -> serde_json::Result<Settings> {
    let mut value: Value = serde_json::from_slice(bytes)?;
    if let Value::Object(obj) = &mut value {
        obj.entry("onboardingDone").or_insert(Value::Bool(true));
    }
    serde_json::from_value(value)
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
    fn old_file_without_llm_fields_loads() {
        let path = temp_path("old");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, br#"{"sttProvider":"deepgram","myLanguage":"en"}"#).unwrap();
        let s = SettingsStore::load(path.clone()).get();
        assert_eq!(s.my_language, Language::En);
        assert_eq!(s.llm_provider, LlmProvider::Ollama);
        assert_eq!(s.stt_engine, SttEngineKind::Deepgram);
        assert!(!s.local_only);
        assert!(s.nudges);
        // Già configurato prima del setup guidato: niente setup.
        assert!(s.onboarding_done);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn onboarding_on_first_run_only() {
        let path = temp_path("onboarding");
        let store = SettingsStore::load(path.clone());
        assert!(!store.get().onboarding_done);
        // Salvato a metà setup: al riavvio il setup riparte.
        store.update(json!({"myLanguage": "en"})).unwrap();
        assert!(!SettingsStore::load(path.clone()).get().onboarding_done);
        store.update(json!({"onboardingDone": true})).unwrap();
        assert!(SettingsStore::load(path.clone()).get().onboarding_done);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn stt_engine_and_local_only_patch() {
        let path = temp_path("engine");
        let store = SettingsStore::load(path.clone());
        let s = store
            .update(json!({"sttEngine": "local", "localOnly": true}))
            .unwrap();
        assert_eq!(s.stt_engine, SttEngineKind::Local);
        assert!(s.local_only);
        assert_eq!(s.local_stt_model, sv_models::PARAKEET_V3);
        assert_eq!(SettingsStore::load(path.clone()).get(), s);
        assert!(store.update(json!({"sttEngine": "whisper"})).is_err());
        // La vecchia chiave non è più accettata nei patch.
        assert!(store.update(json!({"sttProvider": "deepgram"})).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn llm_patch_and_target_language() {
        let store = SettingsStore::load(temp_path("llm"));
        let s = store
            .update(json!({"llmProvider": "opencode", "translationModel": "glm-5.3"}))
            .unwrap();
        assert_eq!(s.llm_provider, LlmProvider::Opencode);
        assert_eq!(s.translation_model, "glm-5.3");
        assert_eq!(s.effective_target_language(), Language::It);
        let s = store.update(json!({"targetLanguage": "en"})).unwrap();
        assert_eq!(s.effective_target_language(), Language::En);
        let s = store
            .update(json!({"targetLanguage": null, "myLanguage": "multi"}))
            .unwrap();
        assert_eq!(s.effective_target_language(), Language::It);
        assert!(store.update(json!({"llmProvider": "gemini"})).is_err());
        let s = store.update(json!({"llmProvider": "opencode-go"})).unwrap();
        assert_eq!(s.llm_provider, LlmProvider::OpencodeGo);
        assert_eq!(s.llm_provider.key_account(), Some("opencode"));
        let s = store.update(json!({"llmProvider": "anthropic"})).unwrap();
        assert_eq!(s.llm_provider.key_account(), Some("anthropic"));
        let s = store.update(json!({"llmProvider": "openai"})).unwrap();
        assert_eq!(s.llm_provider, LlmProvider::Openai);
        let _ = std::fs::remove_dir_all(temp_path("llm").parent().unwrap());
    }

    #[test]
    fn task_providers_default_to_main() {
        let s = Settings {
            llm_provider: LlmProvider::Opencode,
            translation_model: "small".into(),
            ..Settings::default()
        };
        assert!(!s.auto_translate);
        let tr = s.translation_llm().unwrap();
        assert_eq!(
            (tr.provider, tr.model.as_str()),
            (LlmProvider::Opencode, "small")
        );
        assert_eq!(s.assist_llm(), None);
        assert_eq!(s.insight_llm(), Some(tr));
        let s = Settings {
            assist_model: "big".into(),
            assist_provider: Some(LlmProvider::Anthropic),
            ..s
        };
        assert_eq!(s.insight_llm().unwrap().provider, LlmProvider::Anthropic);
        assert_eq!(s.translation_llm().unwrap().provider, LlmProvider::Opencode);
    }

    #[test]
    fn llm_base_url() {
        assert!(LlmProvider::Ollama.base_url("").unwrap().contains("11434"));
        assert_eq!(
            LlmProvider::Custom.base_url(" https://x.dev/v1 ").unwrap(),
            "https://x.dev/v1"
        );
        assert!(LlmProvider::Custom.base_url("").is_err());
        assert!(LlmProvider::Custom.base_url("ftp://x").is_err());
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
