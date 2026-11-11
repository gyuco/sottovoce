//! Scelta del motore di trascrizione e vincoli della modalità "solo locale".
//!
//! Due passi: [`resolve`] decide dai settings quale motore usare e verifica
//! che sia pronto (modelli scaricati, chiave presente), senza caricare nulla;
//! [`EngineSpec::build`] crea il motore. Così la scelta è testabile senza
//! modelli né rete.

use std::path::PathBuf;

use anyhow::{Result, bail};
use sv_models::{ModelKind, ModelStatus, ModelStore};
use sv_stt::SttEngine;
use sv_stt::deepgram::{Deepgram, DeepgramConfig};
use sv_stt_local::{LocalEngine, LocalEngineConfig, LocalModel};
use sv_vad::VadConfig;

use crate::diarize::DiarConfig;
use crate::settings::{LlmProvider, Settings, SttEngineKind};

/// Motore da creare per la sessione, con tutto ciò che gli serve.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineSpec {
    Local {
        model_id: String,
        model_dir: PathBuf,
        vad_model: PathBuf,
    },
    Deepgram {
        api_key: String,
        model: String,
        /// Parlanti di "Loro" da Deepgram (numero libero). Con un numero di
        /// parlanti fissato resta CAM++ con il raggruppamento a k fisso.
        diarize: bool,
    },
}

impl EngineSpec {
    /// Crea il motore. Il modello locale si carica in background: gli stream
    /// emettono `Connected` quando è pronto.
    pub fn build(self) -> Result<Box<dyn SttEngine>> {
        Ok(match self {
            Self::Local {
                model_id,
                model_dir,
                vad_model,
            } => {
                let model = match model_id.as_str() {
                    sv_models::PARAKEET_V3 => LocalModel::Parakeet { dir: model_dir },
                    other => bail!("modello di trascrizione locale non supportato: {other}"),
                };
                Box::new(LocalEngine::new(LocalEngineConfig {
                    model,
                    vad: VadConfig::new(vad_model),
                    num_threads: inference_threads(),
                }))
            }
            Self::Deepgram {
                api_key,
                model,
                diarize,
            } => Box::new(Deepgram::new(DeepgramConfig {
                api_key,
                model,
                endpoint: None,
                diarize,
            })?),
        })
    }
}

/// Metà dei core (max 4): l'altra metà resta a cattura, VAD e UI.
fn inference_threads() -> i32 {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    (cores / 2).clamp(1, 4) as i32
}

/// Motore effettivo: "solo locale" forza il motore locale.
pub fn engine_kind(settings: &Settings) -> SttEngineKind {
    if settings.local_only {
        SttEngineKind::Local
    } else {
        settings.stt_engine
    }
}

/// Motore della sessione dai settings. `api_key` = chiave Deepgram letta dal
/// Keychain (serve solo se il motore effettivo è Deepgram).
pub fn resolve(
    settings: &Settings,
    models: &ModelStore,
    api_key: Option<String>,
) -> Result<EngineSpec> {
    match engine_kind(settings) {
        SttEngineKind::Deepgram => {
            let Some(api_key) = api_key.filter(|k| !k.trim().is_empty()) else {
                bail!("API key Deepgram mancante: impostala nelle Impostazioni");
            };
            Ok(EngineSpec::Deepgram {
                api_key,
                model: settings.stt_model.clone(),
                diarize: settings.diarization && settings.their_speakers == 0,
            })
        }
        SttEngineKind::Local => {
            let id = settings.local_stt_model.trim();
            let spec = sv_models::find(id)
                .filter(|s| s.kind == ModelKind::Stt)
                .ok_or_else(|| {
                    anyhow::anyhow!("modello di trascrizione locale sconosciuto: {id}")
                })?;
            let vad = sv_models::find(sv_models::SILERO_VAD).expect("Silero VAD nel catalogo");
            let model_dir = ready_dir(models, spec)?;
            let vad_model = ready_dir(models, vad)?.join("silero_vad.onnx");
            Ok(EngineSpec::Local {
                model_id: spec.id.clone(),
                model_dir,
                vad_model,
            })
        }
    }
}

fn ready_dir(models: &ModelStore, spec: &sv_models::ModelSpec) -> Result<PathBuf> {
    let state = match models.status(spec) {
        ModelStatus::Ready => return Ok(models.model_dir(spec)),
        ModelStatus::Missing => "non scaricato",
        ModelStatus::Partial { .. } => "scaricato solo in parte",
        ModelStatus::Corrupt => "danneggiato",
    };
    bail!(
        "modello \"{}\" {state}: scaricalo in Impostazioni → Modelli locali",
        spec.name
    )
}

/// Diarization dai settings: `None` se spenta o se il modello CAM++ non è
/// pronto (la sessione parte comunque, senza parlanti).
pub fn diarization(settings: &Settings, models: &ModelStore) -> Option<DiarConfig> {
    if !settings.diarization {
        return None;
    }
    let spec = sv_models::find(sv_models::CAMPLUS).expect("CAM++ nel catalogo");
    match ready_dir(models, spec) {
        Ok(dir) => Some(DiarConfig {
            model: dir.join(&spec.files[0].name),
            speakers: settings.their_speakers,
            voices: None,
        }),
        Err(e) => {
            tracing::info!("riconoscimento dei parlanti disattivato: {e:#}");
            None
        }
    }
}

/// Vincoli della modalità "solo locale" sul modello linguistico: se
/// traduzione o suggerimenti sono attivi, il provider deve girare su questa
/// macchina (Ollama o endpoint personalizzato su localhost).
pub fn check_local_only(settings: &Settings) -> Result<()> {
    if !settings.local_only {
        return Ok(());
    }
    let all_local = [settings.translation_llm(), settings.assist_llm()]
        .into_iter()
        .flatten()
        .all(|task| is_local_llm(settings, task.provider));
    if all_local {
        return Ok(());
    }
    bail!(
        "modalità \"Solo locale\": il modello linguistico deve girare su questo computer \
         (Ollama o endpoint personalizzato su localhost). Cambia provider oppure disattiva \
         \"Solo locale\" nelle Impostazioni"
    )
}

pub fn is_local_llm(settings: &Settings, provider: LlmProvider) -> bool {
    match provider {
        LlmProvider::Ollama => true,
        LlmProvider::Custom => is_loopback_url(&settings.llm_base_url),
        _ => false,
    }
}

/// `http(s)://` verso localhost o un indirizzo di loopback.
fn is_loopback_url(url: &str) -> bool {
    let url = url.trim();
    let Some(rest) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Le credenziali (`user:pass@`) non fanno parte dell'host.
    let host_port = authority.rsplit('@').next().unwrap_or_default();
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        match v6.split_once(']') {
            Some((h, _)) => h,
            None => return false,
        }
    } else {
        host_port.split(':').next().unwrap_or_default()
    };
    let host = host.to_ascii_lowercase();
    let host = host.trim_end_matches('.');
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "::1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (ModelStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sv-stt-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (ModelStore::new(dir.clone()), dir)
    }

    #[test]
    fn deepgram_needs_key() {
        let (models, _) = store("dg");
        let s = Settings {
            stt_engine: SttEngineKind::Deepgram,
            ..Settings::default()
        };
        assert!(resolve(&s, &models, None).is_err());
        assert!(resolve(&s, &models, Some("  ".into())).is_err());
        let spec = resolve(&s, &models, Some("k".into())).unwrap();
        assert_eq!(
            spec,
            EngineSpec::Deepgram {
                api_key: "k".into(),
                model: "nova-3".into(),
                diarize: s.diarization && s.their_speakers == 0,
            }
        );
    }

    #[test]
    fn local_without_models_is_a_clear_error() {
        let (models, _) = store("local");
        let s = Settings {
            stt_engine: SttEngineKind::Local,
            ..Settings::default()
        };
        let err = resolve(&s, &models, Some("k".into())).unwrap_err();
        assert!(err.to_string().contains("Modelli locali"), "{err}");
    }

    #[test]
    fn local_only_forces_local_engine() {
        let (models, _) = store("forced");
        let s = Settings {
            stt_engine: SttEngineKind::Deepgram,
            local_only: true,
            ..Settings::default()
        };
        assert_eq!(engine_kind(&s), SttEngineKind::Local);
        // Con la chiave Deepgram presente non la usa: chiede i modelli locali.
        assert!(resolve(&s, &models, Some("k".into())).is_err());
    }

    #[test]
    fn unknown_or_wrong_kind_local_model() {
        let (models, _) = store("unknown");
        for id in ["nope", sv_models::SILERO_VAD] {
            let s = Settings {
                stt_engine: SttEngineKind::Local,
                local_stt_model: id.into(),
                ..Settings::default()
            };
            let err = resolve(&s, &models, None).unwrap_err();
            assert!(err.to_string().contains("sconosciuto"), "{err}");
        }
    }

    #[test]
    fn diarization_off_or_without_model() {
        let (models, _) = store("diar");
        let on = Settings::default();
        assert!(on.diarization);
        assert!(diarization(&on, &models).is_none(), "modello assente");
        let off = Settings {
            diarization: false,
            ..Settings::default()
        };
        assert!(diarization(&off, &models).is_none());
    }

    #[test]
    fn local_only_llm_constraint() {
        let base = Settings {
            local_only: true,
            translation_model: "m".into(),
            ..Settings::default()
        };
        assert!(check_local_only(&base).is_ok(), "Ollama è locale");

        let cloud = Settings {
            llm_provider: LlmProvider::Openai,
            ..base.clone()
        };
        assert!(check_local_only(&cloud).is_err());
        // LLM non usato (nessun modello scelto): nessuna connessione, ok.
        let unused = Settings {
            translation_model: String::new(),
            assist_model: " ".into(),
            ..cloud.clone()
        };
        assert!(check_local_only(&unused).is_ok());
        // Fuori da "solo locale" tutto ammesso.
        let off = Settings {
            local_only: false,
            ..cloud
        };
        assert!(check_local_only(&off).is_ok());

        let custom = |url: &str| Settings {
            llm_provider: LlmProvider::Custom,
            llm_base_url: url.into(),
            ..base.clone()
        };
        assert!(check_local_only(&custom("http://localhost:1234/v1")).is_ok());
        assert!(check_local_only(&custom("https://api.groq.com/openai/v1")).is_err());
        // Ogni compito col proprio provider: basta uno esterno per bloccare.
        let mixed = Settings {
            assist_model: "m".into(),
            assist_provider: Some(LlmProvider::Anthropic),
            ..base.clone()
        };
        assert!(check_local_only(&mixed).is_err());
        let main_cloud_task_local = Settings {
            llm_provider: LlmProvider::Openai,
            translation_provider: Some(LlmProvider::Ollama),
            ..base.clone()
        };
        assert!(check_local_only(&main_cloud_task_local).is_ok());
    }

    #[test]
    fn loopback_urls() {
        for ok in [
            "http://localhost:1234/v1",
            "http://LOCALHOST/v1",
            "http://127.0.0.1:8080",
            "http://127.1.2.3/v1",
            "http://[::1]:11434/v1",
            "http://ollama.localhost/v1",
            " http://user:pw@localhost:1/v1 ",
        ] {
            assert!(is_loopback_url(ok), "{ok}");
        }
        for bad in [
            "",
            "localhost:1234",
            "ftp://localhost",
            "https://api.openai.com/v1",
            "http://localhost.evil.com/v1",
            "http://evil.com/localhost",
            "http://localhost@evil.com/v1",
            "http://192.168.1.10:11434/v1",
            "http://[::2]/v1",
            "http://[::1/v1",
        ] {
            assert!(!is_loopback_url(bad), "{bad}");
        }
    }
}
