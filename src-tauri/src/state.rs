use tokio::sync::Mutex;

use crate::session::Session;

#[derive(Default)]
pub struct AppState {
    /// Sessione attiva. Mutex async: start/stop attendono la rete.
    pub session: Mutex<Option<Session>>,
}
