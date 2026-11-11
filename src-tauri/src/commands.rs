use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};

use serde::Serialize;
use sv_audio::{InputDevice, LevelEvent};
use sv_llm::{ChatRequest, Message, ModelInfo};
use sv_store::{MeetingDraft, Writer};
use sv_stt::{Language, SttEvent};

use crate::assist::{AssistConfig, AssistEvent};
use crate::chat::{self, ChatInput, ChatTurn};
use crate::codebase::{self, CodePathInfo};
use crate::compress::{CompressConfig, CompressionEvent};
use crate::diarize::{VoiceEvent, VoiceProfiles};
use crate::export;
use crate::history_chat::{self, HistoryAnswer};
use crate::llm;
use crate::meeting::MeetingContext;
use crate::models::{self, LocalModel, Models};
use crate::search::{HistoryFilter, IndexStatus, Search, SearchResult};
use crate::secrets;
use crate::session::{LlmTasks, Outputs};
use crate::session::{Session, SummaryJob};
use crate::settings::{self, Settings, SettingsStore, SttEngineKind};
use crate::state::{AppState, Db, Voices};
use crate::stt;
use crate::summary::SummaryConfig;
use crate::translate::{MeetingLanguage, TranslationEvent, TranslatorConfig};
use crate::update;
use crate::widget;

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

/// Versione e build in esecuzione (Impostazioni → Informazioni).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: String,
    /// Sha corto del commit; `-dirty` = modifiche non committate.
    commit: &'static str,
    /// Ora del build, RFC 3339 UTC.
    built_at: &'static str,
    debug: bool,
}

#[tauri::command]
pub fn app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        commit: env!("SV_INFO_COMMIT"),
        built_at: env!("SV_INFO_BUILT"),
        debug: cfg!(debug_assertions),
    }
}

/// TEMPORANEO: log degli errori JS del webview.
#[tauri::command]
pub fn js_error(message: String) {
    eprintln!("JS-ERROR: {message}");
}

/// Chiude l'applicazione (voce "Esci" dell'icona nel menu bar).
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Mostra o nasconde il widget overlay; ritorna `true` se ora è aperto.
/// Async: su Windows creare una finestra da un comando sincrono blocca.
#[tauri::command]
pub async fn toggle_widget(app: AppHandle) -> CmdResult<bool> {
    widget::toggle(&app).map_err(to_msg)
}

/// Chiude il widget (pulsante nel widget stesso).
#[tauri::command]
pub async fn close_widget(app: AppHandle) {
    widget::close(&app);
}

/// Avvia una sessione (ferma quella eventualmente attiva) con le impostazioni
/// correnti. Livelli su `on_level`, trascrizione su `on_transcript`.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // stato Tauri + un canale IPC per flusso
pub async fn start_session(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: State<'_, SettingsStore>,
    models: State<'_, Models>,
    db: State<'_, Db>,
    voices: State<'_, Voices>,
    on_level: Channel<LevelEvent>,
    on_transcript: Channel<SttEvent>,
    on_translation: Channel<TranslationEvent>,
    on_assist: Channel<AssistEvent>,
    on_compression: Channel<CompressionEvent>,
    on_voice: Channel<VoiceEvent>,
    context: Option<MeetingContext>,
    language: Option<Language>,
) -> CmdResult<()> {
    let mut slot = state.session.lock().await;
    if let Some(old) = slot.take() {
        // Riavvio a sessione attiva (es. cambio microfono): il riepilogo
        // del meeting appena chiuso non va perso.
        if let Some(job) = old.stop().await.summary {
            spawn_summary(app.clone(), db.path.clone(), job);
        }
    }
    let mut settings = settings.get();
    // Lingua della call scelta all'avvio: la parlano tutti, quindi vale per
    // il rilevamento di "Loro" e di "Io". La lingua dell'utente resta quella
    // di arrivo di traduzioni e riepilogo.
    if let Some(language) = language {
        settings.their_language = language;
    }
    let my_spoken = language.unwrap_or(settings.my_language);
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
        my_language: my_spoken.code().into(),
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
    let mut diarization = stt::diarization(&settings, models.store());
    if settings.voice_profiles
        && let Some(cfg) = diarization.as_mut()
    {
        let store = voices.0.clone();
        let loaded = store.clone();
        match blocking(move || loaded.known().map_err(anyhow::Error::from)).await {
            Ok(known) => cfg.voices = Some(VoiceProfiles { store, known }),
            Err(e) => tracing::warn!("profili vocali non disponibili: {e}"),
        }
    }
    let mut llm = llm_tasks(&settings).await;
    if language.is_some() {
        // Dopo `llm_tasks`: le traduzioni restano nella lingua dell'utente;
        // da qui `my_language` è solo la lingua parlata da "Io".
        if let Some(t) = llm.translation.as_mut() {
            t.my_hint = Some(my_spoken).filter(|l| *l != Language::Multi);
        }
        settings.my_language = my_spoken;
    }
    // Interruttore condiviso: `update_settings` lo cambia a call in corso.
    state
        .assist_on
        .store(settings.live_assist, Ordering::Relaxed);
    if let Some(a) = llm.assist.as_mut() {
        a.enabled = state.assist_on.clone();
    }
    // Lingua della call fissa: suggerimenti, riepilogo progressivo e finale
    // nella lingua della call. "Automatica" = lingua dell'utente.
    if let Some(call) = language.filter(|l| *l != Language::Multi) {
        if let Some(a) = llm.assist.as_mut() {
            a.user_language = call;
        }
        if let Some(c) = llm.compression.as_mut() {
            c.language = call;
        }
        if let Some(s) = llm.summary.as_mut() {
            s.language = call;
        }
    }
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
        on_compression: Box::new(move |e| {
            let _ = on_compression.send(e);
        }),
        on_voice: std::sync::Arc::new(move |e| {
            let _ = on_voice.send(e);
        }),
    };
    let session = Session::start(&settings, engine, context, llm, diarization, store, out)
        .map_err(|e| format!("{e:#}"))?;
    *slot = Some(session);
    Ok(())
}

/// Verifica una cartella di codice locale: esistenza, numero di file e
/// linguaggi. Sola lettura, nessun indice persistente.
#[tauri::command]
pub async fn inspect_code_path(path: String) -> CmdResult<CodePathInfo> {
    tauri::async_runtime::spawn_blocking(move || codebase::inspect(&path))
        .await
        .map_err(to_msg)
}

/// Domanda libera al modello durante la call. Il contesto e la trascrizione
/// recente arrivano dalla UI (che li ha già); il codice locale lo legge qui
/// `codebase.rs`, in sola lettura.
#[tauri::command]
pub async fn ask_model(
    settings: State<'_, SettingsStore>,
    context: MeetingContext,
    question: String,
    history: Vec<ChatTurn>,
    transcript: String,
) -> CmdResult<String> {
    let settings = settings.get();
    // "Solo locale" vale anche per la chat.
    stt::check_local_only(&settings).map_err(|e| format!("{e:#}"))?;
    let task = settings.insight_llm().ok_or_else(|| {
        "nessun modello scelto per la chat: impostane uno nelle Impostazioni".to_owned()
    })?;
    let question = question.trim().to_owned();
    if question.is_empty() {
        return Err("domanda vuota".into());
    }

    let context = context.normalized();
    let terms = codebase::extract_terms(
        std::iter::once(question.as_str()).chain(transcript.lines()),
        8,
    );
    let code_path = context.code_path.clone();
    let code = if code_path.is_empty() {
        None
    } else {
        tauri::async_runtime::spawn_blocking(move || {
            codebase::Codebase::open(&code_path)
                .ok()
                .and_then(|base| base.context_block(&terms))
        })
        .await
        .map_err(to_msg)?
    };

    let language = crate::compress::language_name(settings.summary_language());
    let messages = chat::messages(
        &context,
        ChatInput {
            question,
            history,
            transcript,
            language,
            code,
        },
    );
    let provider = task.provider;
    let client = {
        let settings = settings.clone();
        blocking(move || llm::client(&settings, provider, true)).await?
    };
    let request = ChatRequest::new(task.model, messages)
        .temperature(0.4)
        .max_tokens(700)
        .timeout(chat::TIMEOUT);
    let reply = client
        .chat(request)
        .await
        .map_err(|e| format!("risposta del modello non riuscita: {e}"))?;
    let reply = reply.content.trim().to_owned();
    if reply.is_empty() {
        return Err("il modello ha risposto vuoto".into());
    }
    Ok(reply)
}

/// Domanda sullo storico (RAG): recupera i passaggi dei meeting salvati e
/// chiede al modello una risposta solo da quelli, con citazioni `[n]`. Senza
/// passaggi risponde "non trovato" senza chiamare il modello.
#[tauri::command]
pub async fn ask_history(
    search: State<'_, Search>,
    settings: State<'_, SettingsStore>,
    question: String,
    history: Vec<ChatTurn>,
    filter: Option<HistoryFilter>,
    utc_offset_min: i32,
) -> CmdResult<HistoryAnswer> {
    let settings = settings.get();
    // "Solo locale" vale anche qui: i passaggi lasciano il Mac solo verso
    // un LLM locale.
    stt::check_local_only(&settings).map_err(|e| format!("{e:#}"))?;
    let task = settings.insight_llm().ok_or_else(|| {
        "nessun modello scelto per la chat: impostane uno nelle Impostazioni".to_owned()
    })?;
    let question = question.trim().to_owned();
    if question.is_empty() {
        return Err("domanda vuota".into());
    }
    // Fuso orario plausibile (±14 h): solo per le date nel contesto.
    let utc_offset_min = utc_offset_min.clamp(-14 * 60, 14 * 60);

    let query = history_chat::search_query(&question, &history);
    let retrieved = search
        .retrieve(query, filter.unwrap_or_default(), utc_offset_min)
        .await
        .map_err(|e| format!("{e:#}"))?;
    if retrieved.context.sources.is_empty() {
        return Ok(history_chat::not_found(retrieved.semantic));
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    let today = sv_search::date(now_ms, utc_offset_min);
    let messages = history_chat::messages(&question, &history, &retrieved.context.text, &today);
    let provider = task.provider;
    let client = {
        let settings = settings.clone();
        blocking(move || llm::client(&settings, provider, true)).await?
    };
    let request = ChatRequest::new(task.model, messages)
        .temperature(0.2)
        .max_tokens(700)
        .timeout(history_chat::TIMEOUT);
    let reply = client
        .chat(request)
        .await
        .map_err(|e| format!("risposta del modello non riuscita: {e}"))?;
    if reply.content.trim().is_empty() {
        return Err("il modello ha risposto vuoto".into());
    }
    Ok(history_chat::answer(
        &reply.content,
        &retrieved.context.sources,
        retrieved.semantic,
    ))
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
/// Ogni compito usa il proprio provider; un client per provider (per OpenCode
/// Go: un id di sessione).
async fn llm_tasks(settings: &Settings) -> LlmTasks {
    let translation = settings.translation_llm();
    let assist = settings.assist_llm();
    let insight = settings.insight_llm();
    if translation.is_none() && assist.is_none() {
        tracing::info!("traduzione e live assist disattivati: nessun modello scelto");
        return LlmTasks::default();
    }
    let mut clients: Vec<(
        settings::LlmProvider,
        Option<std::sync::Arc<dyn sv_llm::LlmProvider>>,
    )> = Vec::new();
    for task in [&translation, &assist].into_iter().flatten() {
        if clients.iter().any(|(p, _)| *p == task.provider) {
            continue;
        }
        let (for_client, provider) = (settings.clone(), task.provider);
        let client = match blocking(move || llm::client(&for_client, provider, true)).await {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!(?provider, "modello linguistico non disponibile: {e}");
                None
            }
        };
        clients.push((task.provider, client));
    }
    let client_for = |task: &Option<settings::TaskModel>| {
        let task = task.as_ref()?;
        let client = clients
            .iter()
            .find(|(p, _)| *p == task.provider)
            .and_then(|(_, c)| c.clone())?;
        Some((client, task.model.clone()))
    };
    let language = settings.effective_target_language();
    let insight_language = settings.summary_language();
    LlmTasks {
        translation: client_for(&translation).map(|(llm, model)| TranslatorConfig {
            llm,
            model,
            target: language,
            source_hint: Some(settings.their_language).filter(|l| *l != Language::Multi),
            my_hint: Some(settings.my_language).filter(|l| *l != Language::Multi),
            meeting: MeetingLanguage::new(settings.their_language),
        }),
        auto_translate: settings.auto_translate,
        assist: client_for(&assist).map(|(llm, model)| AssistConfig {
            llm,
            model,
            user_language: language,
            enabled: Arc::new(AtomicBool::new(settings.live_assist)),
        }),
        compression: client_for(&insight).map(|(llm, model)| CompressConfig {
            llm,
            model,
            language: insight_language,
        }),
        summary: client_for(&insight).map(|(llm, model)| SummaryConfig {
            llm,
            model,
            language: insight_language,
        }),
        auto_summary: settings.auto_summary,
    }
}

/// Traduce su richiesta una frase della sessione attiva (pulsante
/// "Traduci") e la salva. `previous` = frasi precedenti, solo contesto.
#[tauri::command]
pub async fn translate_segment(
    state: State<'_, AppState>,
    settings: State<'_, SettingsStore>,
    segment_id: u64,
    channel: String,
    text: String,
    previous: Vec<String>,
) -> CmdResult<TranslationEvent> {
    let channel = match channel.as_str() {
        "me" => sv_audio::Channel::Me,
        "them" => sv_audio::Channel::Them,
        other => return Err(format!("canale sconosciuto: {other}")),
    };
    let active = state
        .session
        .lock()
        .await
        .as_ref()
        .map(Session::on_demand_translation);
    let job = match active {
        Some(Some(job)) => job,
        Some(None) => return Err("scegli un modello di traduzione nelle Impostazioni".into()),
        // Sessione finita: si traduce comunque, senza salvare.
        None => {
            let settings = settings.get();
            stt::check_local_only(&settings).map_err(|e| format!("{e:#}"))?;
            let task = settings
                .translation_llm()
                .ok_or_else(|| "scegli un modello di traduzione nelle Impostazioni".to_owned())?;
            let (for_client, provider) = (settings.clone(), task.provider);
            let llm = blocking(move || llm::client(&for_client, provider, true)).await?;
            crate::session::OnDemandTranslation {
                cfg: TranslatorConfig {
                    llm,
                    model: task.model,
                    target: settings.effective_target_language(),
                    source_hint: None,
                    my_hint: None,
                    // Senza sessione la lingua del meeting è nota solo se impostata.
                    meeting: MeetingLanguage::new(settings.their_language),
                },
                context: None,
                store: None,
            }
        }
    };
    let target = job.cfg.target_for(channel)?;
    let (translated, latency_ms) = crate::translate::translate_one(
        &job.cfg,
        &target,
        job.context.as_deref(),
        &previous,
        &text,
    )
    .await?;
    if let Some(store) = &job.store {
        store.translation(segment_id, &translated, &target, latency_ms);
    }
    Ok(TranslationEvent {
        segment_id,
        text: translated,
        language: target,
        latency_ms,
    })
}

/// Esito dello stop per la UI: il riepilogo, se previsto, arriva dopo con
/// l'evento [`SUMMARY_EVENT`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopResult {
    pub meeting_id: Option<i64>,
    pub summary_pending: bool,
}

/// Evento con l'esito del riepilogo finale generato in background.
pub const SUMMARY_EVENT: &str = "summary-status";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
enum SummaryStatus {
    #[serde(rename_all = "camelCase")]
    Ready {
        meeting_id: i64,
        summary: sv_store::Summary,
    },
    #[serde(rename_all = "camelCase")]
    Failed { meeting_id: i64, message: String },
}

#[tauri::command]
pub async fn stop_session(
    app: AppHandle,
    state: State<'_, AppState>,
    db: State<'_, Db>,
) -> CmdResult<StopResult> {
    let session = state.session.lock().await.take();
    let Some(session) = session else {
        return Ok(StopResult {
            meeting_id: None,
            summary_pending: false,
        });
    };
    let stopped = session.stop().await;
    if let Some(id) = stopped.meeting_id {
        app.state::<Search>().enqueue(id);
    }
    let summary_pending = stopped.summary.is_some();
    if let Some(job) = stopped.summary {
        spawn_summary(app, db.path.clone(), job);
    }
    Ok(StopResult {
        meeting_id: stopped.meeting_id,
        summary_pending,
    })
}

/// Genera il riepilogo finale senza bloccare la UI; l'esito va all'evento
/// [`SUMMARY_EVENT`]. Si può intanto avviare un'altra sessione.
fn spawn_summary(app: AppHandle, db_path: PathBuf, job: SummaryJob) {
    tauri::async_runtime::spawn(async move {
        let meeting_id = job.meeting_id;
        let status = match job.run(db_path).await {
            Ok(summary) => {
                app.state::<Search>().enqueue(meeting_id);
                SummaryStatus::Ready {
                    meeting_id,
                    summary,
                }
            }
            Err(e) => {
                tracing::warn!(meeting = meeting_id, "riepilogo finale non riuscito: {e:#}");
                SummaryStatus::Failed {
                    meeting_id,
                    message: format!("{e:#}"),
                }
            }
        };
        if let Err(e) = app.emit(SUMMARY_EVENT, status) {
            tracing::warn!("evento riepilogo non inviato: {e}");
        }
    });
}

// --- Storico meeting ---------------------------------------------------------

/// Pagina dello storico, più recenti prima. `search` filtra sulle frasi lette
/// dai file di trascrizione (il testo non è nel database).
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

/// Ricerca nello storico per significato e per parole (Reciprocal Rank
/// Fusion). Senza il modello di embedding restano le sole parole.
#[tauri::command]
pub async fn search_meetings(
    search: State<'_, Search>,
    query: String,
) -> CmdResult<Vec<SearchResult>> {
    search.search(query).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn semantic_index_status(search: State<'_, Search>) -> CmdResult<IndexStatus> {
    let search = search.inner().clone_handle();
    blocking(move || Ok(search.status())).await
}

/// Ricontrolla tutti i meeting e indicizza quelli senza indice o cambiati.
#[tauri::command]
pub fn reindex_meetings(search: State<'_, Search>) {
    search.backfill();
}

#[tauri::command]
pub async fn delete_meeting(
    db: State<'_, Db>,
    search: State<'_, Search>,
    id: i64,
) -> CmdResult<()> {
    let path = db.path.clone();
    blocking(move || Ok(sv_store::Store::open(path)?.delete_meeting(id)?)).await?;
    // I vettori permettono di ricostruire parte del testo: vanno via insieme.
    search.remove(id);
    Ok(())
}

#[tauri::command]
pub async fn rename_meeting(
    db: State<'_, Db>,
    search: State<'_, Search>,
    id: i64,
    name: String,
) -> CmdResult<sv_store::Meeting> {
    let path = db.path.clone();
    let meeting =
        blocking(move || Ok(sv_store::Store::open(path)?.rename_meeting(id, &name)?)).await?;
    // Il nome è il titolo dei documenti indicizzati.
    search.enqueue(id);
    Ok(meeting)
}

/// Rigenera il riepilogo finale di un meeting salvato (anche in un'altra lingua).
#[tauri::command]
pub async fn generate_summary(
    db: State<'_, Db>,
    search: State<'_, Search>,
    settings: State<'_, SettingsStore>,
    id: i64,
    language: Option<Language>,
) -> CmdResult<sv_store::Summary> {
    let settings = settings.get();
    stt::check_local_only(&settings).map_err(|e| format!("{e:#}"))?;
    let task = settings
        .insight_llm()
        .ok_or_else(|| "scegli un modello per il live assist o la traduzione".to_owned())?;
    let language = match language.unwrap_or(settings.summary_language()) {
        Language::Multi => Language::It,
        l => l,
    };
    let path = db.path.clone();
    let details = blocking(move || Ok(sv_store::Store::open(path)?.details(id)?)).await?;
    if details.segments.is_empty() && details.blocks.is_empty() {
        return Err("nessuna trascrizione per questo meeting".into());
    }

    let speakers = details
        .speakers
        .iter()
        .map(|s| (s.speaker, s.name.clone()))
        .collect();
    let context = MeetingContext {
        name: details.meeting.name.clone(),
        goal: details.meeting.goal.clone(),
        notes: details.meeting.notes.clone(),
        // Il percorso del codice non è salvato nello storico.
        code_path: String::new(),
        speakers,
    };
    let blocks = details
        .blocks
        .iter()
        .map(|b| CompressionEvent {
            index: b.index,
            start: b.start_ms,
            end: b.end_ms,
            summary: b.summary.clone(),
            topics: b.topics.clone(),
            moments: b.moments.clone(),
        })
        .collect();
    let lines = details
        .segments
        .iter()
        .rev()
        .take(80)
        .rev()
        .map(|s| crate::compress::CompressLine {
            channel: if s.channel == "me" {
                sv_audio::Channel::Me
            } else {
                sv_audio::Channel::Them
            },
            text: s.text.clone(),
            speaker: s.speaker,
            start_ms: s.start_ms,
            end_ms: s.end_ms,
        })
        .collect();
    let provider = task.provider;
    let client = blocking(move || llm::client(&settings, provider, true)).await?;
    let cfg = SummaryConfig {
        llm: client,
        model: task.model,
        language,
    };
    let input = crate::summary::SummaryInput {
        context,
        blocks,
        lines,
    };
    let draft = crate::summary::generate(&cfg, &input)
        .await
        .ok_or_else(|| "riepilogo non generato".to_owned())?;
    let path = db.path.clone();
    let summary =
        blocking(move || Ok(sv_store::Store::open(path)?.save_summary(id, &draft)?)).await?;
    search.enqueue(id);
    Ok(summary)
}

#[tauri::command]
pub async fn update_summary_checklist(
    db: State<'_, Db>,
    id: i64,
    items: Vec<sv_store::ActionItem>,
) -> CmdResult<sv_store::Summary> {
    let path = db.path.clone();
    blocking(move || Ok(sv_store::Store::open(path)?.set_action_items(id, &items)?)).await
}

// --- API key (Keychain) ---------------------------------------------------
// Il Keychain può bloccare (prompt di accesso): mai sul thread principale.

/// Salva un meeting in Markdown nel file scelto dall'utente; `None` se annulla.
#[tauri::command]
pub async fn export_meeting_markdown(
    app: AppHandle,
    file_name: String,
    contents: String,
) -> CmdResult<Option<String>> {
    let path = export::save_markdown(&app, &file_name, &contents).await?;
    Ok(path.map(|p| p.display().to_string()))
}

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
    state: State<'_, AppState>,
    settings: State<'_, SettingsStore>,
    search: State<'_, Search>,
    patch: serde_json::Value,
) -> CmdResult<Settings> {
    let was = settings.get().semantic_search;
    let next = settings.update(patch).map_err(|e| format!("{e:#}"))?;
    state.assist_on.store(next.live_assist, Ordering::Relaxed);
    if next.semantic_search != was {
        // Riattivata: recupera i meeting arrivati nel frattempo. Spenta: il
        // thread svuota la coda. In entrambi i casi la UI riceve lo stato.
        search.backfill();
    }
    Ok(next)
}

// --- Profili vocali -----------------------------------------------------------

#[tauri::command]
pub async fn list_voice_profiles(
    voices: State<'_, Voices>,
) -> CmdResult<Vec<sv_diar::voices::VoiceProfile>> {
    let store = voices.0.clone();
    blocking(move || store.list().map_err(anyhow::Error::from)).await
}

#[tauri::command]
pub async fn rename_voice_profile(
    voices: State<'_, Voices>,
    id: String,
    name: String,
) -> CmdResult<sv_diar::voices::VoiceProfile> {
    let store = voices.0.clone();
    blocking(move || store.rename(&id, &name).map_err(anyhow::Error::from)).await
}

#[tauri::command]
pub async fn delete_voice_profile(voices: State<'_, Voices>, id: String) -> CmdResult<()> {
    let store = voices.0.clone();
    blocking(move || store.delete(&id).map_err(anyhow::Error::from)).await
}

#[tauri::command]
pub async fn delete_all_voice_profiles(voices: State<'_, Voices>) -> CmdResult<()> {
    let store = voices.0.clone();
    blocking(move || store.delete_all().map_err(anyhow::Error::from)).await
}

// --- LLM ---------------------------------------------------------------------

/// Modelli di `provider` (default: quello principale), con il flag di
/// compatibilità.
#[tauri::command]
pub async fn list_llm_models(
    settings: State<'_, SettingsStore>,
    provider: Option<settings::LlmProvider>,
) -> CmdResult<Vec<ModelInfo>> {
    let settings = settings.get();
    let provider = provider.unwrap_or(settings.llm_provider);
    let client = blocking(move || llm::client(&settings, provider, false)).await?;
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

/// Chiamata minima a `model` su `provider` (default: il modello di
/// traduzione, poi quello dei suggerimenti, col loro provider) per
/// verificare endpoint, chiave e modello.
#[tauri::command]
pub async fn test_llm(
    settings: State<'_, SettingsStore>,
    model: Option<String>,
    provider: Option<settings::LlmProvider>,
) -> CmdResult<LlmTestResult> {
    let settings = settings.get();
    let task = match model.filter(|m| !m.trim().is_empty()) {
        Some(model) => settings::TaskModel {
            provider: provider.unwrap_or(settings.llm_provider),
            model,
        },
        None => settings
            .translation_llm()
            .or_else(|| settings.assist_llm())
            .ok_or_else(|| "scegli prima un modello".to_owned())?,
    };
    let (provider, model) = (task.provider, task.model);
    let client = blocking(move || llm::client(&settings, provider, true)).await?;
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
    search: State<'_, Search>,
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
    if spec.id == sv_models::EMBEDDING_GEMMA_2 {
        search.model_changed();
    }
    Ok(models.describe(spec))
}

#[tauri::command]
pub fn cancel_local_model_download(models: State<'_, Models>, id: String) -> bool {
    models.cancel(&id)
}

#[tauri::command]
pub async fn delete_local_model(
    models: State<'_, Models>,
    search: State<'_, Search>,
    id: String,
) -> CmdResult<LocalModel> {
    let spec = models::spec(&id).map_err(to_msg)?;
    // Il guard impedisce un download concorrente durante l'eliminazione.
    let guard = models
        .begin(&id)
        .ok_or_else(|| format!("download di {id} in corso: annullalo prima"))?;
    models.store().delete(spec).await.map_err(to_msg)?;
    drop(guard);
    if spec.id == sv_models::EMBEDDING_GEMMA_2 {
        search.model_changed();
    }
    Ok(models.describe(spec))
}

/// Verifica completa (SHA-256 di tutti i file).
#[tauri::command]
pub async fn verify_local_model(
    models: State<'_, Models>,
    search: State<'_, Search>,
    id: String,
) -> CmdResult<LocalModel> {
    let spec = models::spec(&id).map_err(to_msg)?;
    let guard = models
        .begin(&id)
        .ok_or_else(|| format!("download di {id} in corso"))?;
    models.store().verify(spec).await.map_err(to_msg)?;
    drop(guard);
    if spec.id == sv_models::EMBEDDING_GEMMA_2 {
        search.model_changed();
    }
    Ok(models.describe(spec))
}

// --- Aggiornamenti -----------------------------------------------------------

/// Ultima release pubblicata più recente di questa build (`null` se nessuna).
#[tauri::command]
pub async fn check_update() -> Option<update::UpdateInfo> {
    update::check().await
}

/// Apre nel browser la pagina di una release del repo dell'app.
#[tauri::command]
pub fn open_release_page(url: String) -> CmdResult<()> {
    update::open_release(&url).map_err(to_msg)
}
