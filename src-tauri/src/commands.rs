use tauri::State;
use tauri::ipc::Channel;

use serde::Serialize;
use sv_audio::{InputDevice, LevelEvent};
use sv_llm::{ChatRequest, Message, ModelInfo};
use sv_stt::{Language, SttEvent};

use crate::assist::{AssistConfig, AssistEvent};
use crate::llm;
use crate::meeting::MeetingContext;
use crate::secrets;
use crate::session::Session;
use crate::session::{LlmTasks, Outputs};
use crate::settings::{Settings, SettingsStore};
use crate::state::AppState;
use crate::translate::{TranslationEvent, TranslatorConfig};

/// Errore restituito al frontend come stringa leggibile.
type CmdResult<T> = Result<T, String>;

fn to_msg(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn list_input_devices() -> CmdResult<Vec<InputDevice>> {
    sv_audio::list_input_devices().map_err(to_msg)
}

#[tauri::command]
pub fn system_audio_supported() -> bool {
    sv_audio::system::is_supported()
}

/// Avvia una sessione (ferma quella eventualmente attiva) con le impostazioni
/// correnti. Livelli su `on_level`, trascrizione su `on_transcript`.
#[tauri::command]
pub async fn start_session(
    state: State<'_, AppState>,
    settings: State<'_, SettingsStore>,
    on_level: Channel<LevelEvent>,
    on_transcript: Channel<SttEvent>,
    on_translation: Channel<TranslationEvent>,
    on_assist: Channel<AssistEvent>,
    context: Option<MeetingContext>,
) -> CmdResult<()> {
    let mut slot = state.session.lock().await;
    if let Some(old) = slot.take() {
        old.stop().await;
    }
    let settings = settings.get();
    let provider = settings.stt_provider.id();
    let api_key = blocking(move || secrets::get_api_key(provider))
        .await?
        .ok_or_else(|| format!("API key mancante per {provider}: impostala nelle Impostazioni"))?;
    let llm = llm_tasks(&settings).await;
    // Errore di invio = finestra chiusa o canale rilasciato: niente da fare.
    let out = Outputs {
        on_level: Box::new(move |e| {
            let _ = on_level.send(e);
        }),
        on_transcript: std::sync::Arc::new(move |e| {
            let _ = on_transcript.send(e);
        }),
        on_translation: Box::new(move |e| {
            let _ = on_translation.send(e);
        }),
        on_assist: Box::new(move |e| {
            let _ = on_assist.send(e);
        }),
    };
    let session = Session::start(&settings, api_key, context.unwrap_or_default(), llm, out)
        .map_err(|e| format!("{e:#}"))?;
    *slot = Some(session);
    Ok(())
}

/// Aggiorna il contesto della sessione attiva: vale dalla prossima chiamata
/// LLM. Senza sessione non fa nulla (la UI lo passerà a `start_session`).
#[tauri::command]
pub async fn update_meeting_context(
    state: State<'_, AppState>,
    context: MeetingContext,
) -> CmdResult<()> {
    if let Some(session) = state.session.lock().await.as_ref() {
        session.context().set(context);
    }
    Ok(())
}

/// Traduzione e live assist dalle impostazioni. Un compito senza modello, o
/// un LLM non configurabile, resta spento (con log): la sessione parte comunque.
async fn llm_tasks(settings: &Settings) -> LlmTasks {
    let translation_model = settings.translation_model.trim().to_owned();
    let assist_model = settings.assist_model.trim().to_owned();
    if translation_model.is_empty() && assist_model.is_empty() {
        tracing::info!("traduzione e live assist disattivati: nessun modello scelto");
        return LlmTasks::default();
    }
    let for_client = settings.clone();
    // Un solo client per sessione (per OpenCode Go: un id di sessione).
    let client: std::sync::Arc<dyn sv_llm::LlmProvider> =
        match blocking(move || llm::client(&for_client, true)).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("traduzione e live assist disattivati: {e}");
                return LlmTasks::default();
            }
        };
    LlmTasks {
        translation: (!translation_model.is_empty()).then(|| TranslatorConfig {
            llm: client.clone(),
            model: translation_model,
            target: settings.effective_target_language(),
            source_hint: Some(settings.their_language).filter(|l| *l != Language::Multi),
        }),
        assist: (!assist_model.is_empty()).then(|| AssistConfig {
            llm: client.clone(),
            model: assist_model,
            user_language: settings.effective_target_language(),
        }),
    }
}

#[tauri::command]
pub async fn stop_session(state: State<'_, AppState>) -> CmdResult<()> {
    let session = state.session.lock().await.take();
    if let Some(s) = session {
        s.stop().await;
    }
    Ok(())
}

// --- API key (Keychain) ---------------------------------------------------
// Il Keychain può bloccare (prompt di accesso): mai sul thread principale.

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(to_msg)?
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn set_api_key(provider: String, key: String) -> CmdResult<()> {
    blocking(move || secrets::set_api_key(&provider, &key)).await
}

#[tauri::command]
pub async fn has_api_key(provider: String) -> CmdResult<bool> {
    blocking(move || secrets::has_api_key(&provider)).await
}

#[tauri::command]
pub async fn delete_api_key(provider: String) -> CmdResult<()> {
    blocking(move || secrets::delete_api_key(&provider)).await
}

// --- Impostazioni ----------------------------------------------------------

#[tauri::command]
pub fn get_settings(settings: State<'_, SettingsStore>) -> Settings {
    settings.get()
}

#[tauri::command]
pub fn update_settings(
    settings: State<'_, SettingsStore>,
    patch: serde_json::Value,
) -> CmdResult<Settings> {
    settings.update(patch).map_err(|e| format!("{e:#}"))
}

// --- LLM ---------------------------------------------------------------------

/// Modelli del provider LLM configurato (con il flag di compatibilità).
#[tauri::command]
pub async fn list_llm_models(settings: State<'_, SettingsStore>) -> CmdResult<Vec<ModelInfo>> {
    let settings = settings.get();
    let provider = settings.llm_provider;
    let client = blocking(move || llm::client(&settings, false)).await?;
    let mut models = client
        .list_models()
        .await
        .map_err(|e| llm::describe(provider, &e))?;
    llm::mark_usable(provider, &mut models);
    Ok(models)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmTestResult {
    pub model: String,
    pub latency_ms: u64,
    pub reply: String,
}

/// Chiamata minima al modello `model` (o a quello di traduzione) per
/// verificare endpoint, chiave e modello.
#[tauri::command]
pub async fn test_llm(
    settings: State<'_, SettingsStore>,
    model: Option<String>,
) -> CmdResult<LlmTestResult> {
    let settings = settings.get();
    let provider = settings.llm_provider;
    let model = model
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| settings.translation_model.clone());
    if model.trim().is_empty() {
        return Err("scegli prima un modello".into());
    }
    let client = blocking(move || llm::client(&settings, true)).await?;
    let req = ChatRequest::new(
        model.clone(),
        vec![Message::user("Rispondi solo con la parola: ok")],
    )
    .temperature(0.0);
    let resp = client
        .chat(req)
        .await
        .map_err(|e| llm::describe(provider, &e))?;
    Ok(LlmTestResult {
        model,
        latency_ms: u64::try_from(resp.latency.as_millis()).unwrap_or(u64::MAX),
        reply: resp.content.trim().chars().take(80).collect(),
    })
}
