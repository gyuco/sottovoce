mod commands;
mod secrets;
mod session;
mod settings;
mod state;

use tauri::Manager;

use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                EnvFilter::new("info,sottovoce=debug,sv_audio=debug,sv_stt=debug")
            }),
        )
        .init();

    tauri::Builder::default()
        .manage(state::AppState::default())
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("settings.json");
            app.manage(settings::SettingsStore::load(path));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_input_devices,
            commands::system_audio_supported,
            commands::start_session,
            commands::stop_session,
            commands::set_api_key,
            commands::has_api_key,
            commands::delete_api_key,
            commands::get_settings,
            commands::update_settings,
        ])
        .run(tauri::generate_context!())
        .expect("errore durante l'avvio di Sottovoce");
}
