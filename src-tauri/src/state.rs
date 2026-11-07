use std::path::PathBuf;

use tokio::sync::Mutex;

use crate::session::Session;

#[derive(Default)]
pub struct AppState {
    /// Sessione attiva. Mutex async: start/stop attendono la rete.
    pub session: Mutex<Option<Session>>,
}

/// Percorso del database dei meeting (`app_data_dir/sottovoce.db`).
pub struct Db {
    pub path: PathBuf,
}
