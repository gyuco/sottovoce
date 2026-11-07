use tauri::State;
use tauri::ipc::Channel;

use serde::Serialize;
use sv_audio::{InputDevice, LevelEvent};
use sv_llm::{ChatRequest, Message, ModelInfo};
use sv_store::{MeetingDraft, Writer};
use sv_stt::{Language, SttEvent};

use crate::assist::{AssistConfig, AssistEvent};
use crate::llm;
use crate::meeting::MeetingContext;
use crate::models::{self, LocalModel, Models};
use crate::secrets;
use crate::session::Session;
use crate::session::{LlmTasks, Outputs};
use crate::settings::{self, Settings, SettingsStore, SttEngineKind};
use crate::state::{AppState, Db};
use crate::stt;
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
#[allow(clippy::too_many_arguments)] // stato Tauri + un canale IPC per flusso
pub async fn start_session(
    state: State<'_, AppState>,
    settings: State<'_, SettingsStore>,
    models: State<'_, Models>,
    db: State<'_, Db>,
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
    // Prima di qualsiasi connessione: "solo locale" non ammette LLM esterni.
    stt::check_local_only(&settings).map_err(|e| format!("{e:#}"))?;
    let api_key = match stt::engine_kind(&settings) {
        SttEngineKind::Deepgram => {
            blocking(|| secrets::get_api_key(settings::DEEPGRAM_KEY)).await?
        }
        SttEngineKind::Local => None,
    };
    let spec = stt::resolve(&settings, models.store(), api_key).map_err(|e| format!("{e:#}"))?;
    let engine = spec.build().map_err(|e| format!("{e:#}"))?;
    let context = context.unwrap_or_default().normalized();
    let draft = MeetingDraft {
        name: context.name.clone(),
        goal: context.goal.clone(),
        notes: context.notes.clone(),
        stt_engine: engine.name().into(),
        stt_model: match stt::engine_kind(&settings) {
            SttEngineKind::Deepgram => settings.stt_model.clone(),
            SttEngineKind::Local => settings.local_stt_model.clone(),
        },
        my_language: settings.my_language.code().into(),
        their_language: settings.their_language.code().into(),
        speakers: context.speakers.clone(),
    };
    // Il DB può essere su disco lento: mai sul thread async.
    let db_path = db.path.clone();
    let store =
        match blocking(move || Writer::start(&db_path, draft).map_err(anyhow::Error::from)).await {
            Ok(writer) => Some(writer),
            Err(e) => {
                tracing::warn!("salvataggio del meeting disattivato: {e}");
                None
            }
        };
    let diarization = stt::diarization(&settings, models.store());
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
    let session = Session::start(&settings, engine, context, llm, diarization, store, out)
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
        session.set_context(context);
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

// --- Storico meeting ---------------------------------------------------------

/// Pagina dello storico, più recenti prima. `search` filtra con FTS5 sulle frasi.
#[tauri::command]
pub async fn list_meetings(
    db: State<'_, Db>,
    page: Option<u32>,
    per_page: Option<u32>,
    search: Option<String>,
) -> CmdResult<sv_store::MeetingPage> {
    let path = db.path.clone();
    let per_page = per_page.unwrap_or(20).clamp(1, 100);
    let offset = page.unwrap_or(0).saturating_mul(per_page);
    let limit = per_page;
    blocking(move || {
        let store = sv_store::Store::open(path)?;
        Ok(store.list_meetings(offset, limit, search.as_deref())?)
    })
    .await
}

/// Meeting completo: frasi, traduzioni, parlanti e suggerimenti.
#[tauri::command]
pub async fn get_meeting(db: State<'_, Db>, id: i64) -> CmdResult<sv_store::MeetingDetails> {
    let path = db.path.clone();
    blocking(move || Ok(sv_store::Store::open(path)?.details(id)?)).await
}

#[tauri::command]
pub async fn delete_meeting(db: State<'_, Db>, id: i64) -> CmdResult<()> {
    let path = db.path.clone();
    blocking(move || Ok(sv_store::Store::open(path)?.delete_meeting(id)?)).await
}

#[tauri::command]
pub async fn rename_meeting(
    db: State<'_, Db>,
    id: i64,
    name: String,
) -> CmdResult<sv_store::Meeting> {
    let path = db.path.clone();
    blocking(move || Ok(sv_store::Store::open(path)?.rename_meeting(id, &name)?)).await
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

// --- Modelli locali ------------------------------------------------------------

#[tauri::command]
pub fn list_local_models(models: State<'_, Models>) -> Vec<LocalModel> {
    models.list()
}

/// Scarica (o riprende) un modello; avanzamento su `on_progress`. Ritorna lo
/// stato finale. Annullato → errore "download annullato", parziale conservato.
#[tauri::command]
pub async fn download_local_model(
    models: State<'_, Models>,
    id: String,
    on_progress: Channel<sv_models::DownloadProgress>,
) -> CmdResult<LocalModel> {
    let spec = models::spec(&id).map_err(to_msg)?;
    let guard = models
        .begin(&id)
        .ok_or_else(|| format!("download di {id} già in corso"))?;
    let result = models
        .store()
        .download(
            spec,
            |p| {
                let _ = on_progress.send(p);
            },
            &guard.cancel,
        )
        .await;
    drop(guard);
    result.map_err(to_msg)?;
    Ok(models.describe(spec))
}

#[tauri::command]
pub fn cancel_local_model_download(models: State<'_, Models>, id: String) -> bool {
    models.cancel(&id)
}

#[tauri::command]
pub async fn delete_local_model(models: State<'_, Models>, id: String) -> CmdResult<LocalModel> {
    let spec = models::spec(&id).map_err(to_msg)?;
    // Il guard impedisce un download concorrente durante l'eliminazione.
    let guard = models
        .begin(&id)
        .ok_or_else(|| format!("download di {id} in corso: annullalo prima"))?;
    models.store().delete(spec).await.map_err(to_msg)?;
    drop(guard);
    Ok(models.describe(spec))
}

/// Verifica completa (SHA-256 di tutti i file).
#[tauri::command]
pub async fn verify_local_model(models: State<'_, Models>, id: String) -> CmdResult<LocalModel> {
    let spec = models::spec(&id).map_err(to_msg)?;
    let guard = models
        .begin(&id)
        .ok_or_else(|| format!("download di {id} in corso"))?;
    models.store().verify(spec).await.map_err(to_msg)?;
    drop(guard);
    Ok(models.describe(spec))
}
