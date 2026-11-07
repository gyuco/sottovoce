use tauri::State;
use tauri::ipc::Channel;

use sv_audio::{InputDevice, LevelEvent};
use sv_stt::SttEvent;

use crate::secrets;
use crate::session::Session;
use crate::settings::{Settings, SettingsStore};
use crate::state::AppState;

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
    let session = Session::start(
        &settings,
        api_key,
        // Errore = finestra chiusa o canale rilasciato: niente da fare.
        move |e| {
            let _ = on_level.send(e);
        },
        move |e| {
            let _ = on_transcript.send(e);
        },
    )
    .map_err(|e| format!("{e:#}"))?;
    *slot = Some(session);
    Ok(())
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
