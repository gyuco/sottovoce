mod assist;
mod commands;
mod diarize;
mod llm;
mod meeting;
mod models;
mod secrets;
mod session;
mod settings;
mod state;
mod stt;
mod translate;

use tauri::Manager;

use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new(
                "info,sottovoce=debug,sv_audio=debug,sv_stt=debug,sv_stt_local=debug,sv_diar=debug,sv_llm=debug",
            )
        }))
        .init();

    tauri::Builder::default()
        .manage(state::AppState::default())
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("settings.json");
            app.manage(settings::SettingsStore::load(path));
            let models_dir = app.path().app_data_dir()?.join("models");
            app.manage(models::Models::new(models_dir));
            let db_path = app.path().app_data_dir()?.join("sottovoce.db");
            // Migra e chiude i meeting rimasti aperti dopo un crash.
            match sv_store::Store::open(&db_path) {
                Ok(store) => {
                    tracing::info!(path = %db_path.display(), "database dei meeting pronto");
                    match store.recover_open_meetings() {
                        Ok(0) => {}
                        Ok(n) => tracing::info!(meetings = n, "meeting rimasti aperti chiusi"),
                        Err(e) => tracing::warn!("recupero dei meeting non riuscito: {e}"),
                    }
                }
                Err(e) => tracing::warn!(
                    path = %db_path.display(),
                    "database dei meeting non disponibile, le call non verranno salvate: {e}"
                ),
            }
            app.manage(state::Db { path: db_path });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_input_devices,
            commands::system_audio_supported,
            commands::start_session,
            commands::stop_session,
            commands::update_meeting_context,
            commands::set_api_key,
            commands::has_api_key,
            commands::delete_api_key,
            commands::get_settings,
            commands::update_settings,
            commands::list_llm_models,
            commands::test_llm,
            commands::list_local_models,
            commands::download_local_model,
            commands::cancel_local_model_download,
            commands::delete_local_model,
            commands::verify_local_model,
        ])
        .run(tauri::generate_context!())
        .expect("errore durante l'avvio di Sottovoce");
}
