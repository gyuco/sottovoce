//! Modelli locali (VAD, STT, speaker embedding) nella app data dir:
//! stato, download con avanzamento e annullamento, verifica, eliminazione.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use sv_models::{ModelError, ModelSpec, ModelStatus, ModelStore};

/// Modello del catalogo con il suo stato su disco.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModel {
    #[serde(flatten)]
    pub spec: ModelSpec,
    pub size: u64,
    pub status: ModelStatus,
    /// Byte occupati ora (anche download parziali).
    pub on_disk: u64,
    pub downloading: bool,
}

pub struct Models {
    store: ModelStore,
    /// Download in corso: flag di annullamento per id.
    downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl Models {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            store: ModelStore::new(dir),
            downloads: Mutex::default(),
        }
    }

    pub fn store(&self) -> &ModelStore {
        &self.store
    }

    pub fn list(&self) -> Vec<LocalModel> {
        sv_models::catalog()
            .iter()
            .map(|s| self.describe(s))
            .collect()
    }

    pub fn describe(&self, spec: &ModelSpec) -> LocalModel {
        let downloading = self.is_downloading(&spec.id);
        LocalModel {
            spec: spec.clone(),
            size: spec.size(),
            // Durante il download i file sono a metà: lo stato è "parziale".
            status: self.store.status(spec),
            on_disk: self.store.disk_usage(spec),
            downloading,
        }
    }

    fn is_downloading(&self, id: &str) -> bool {
        self.downloads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
    }

    /// Registra un download; `None` se ce n'è già uno per lo stesso modello.
    pub fn begin(&self, id: &str) -> Option<DownloadGuard<'_>> {
        let mut map = self.downloads.lock().unwrap_or_else(|e| e.into_inner());
        if map.contains_key(id) {
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        map.insert(id.to_owned(), cancel.clone());
        Some(DownloadGuard {
            models: self,
            id: id.to_owned(),
            cancel,
        })
    }

    /// Chiede l'annullamento; `false` se non c'era un download in corso.
    pub fn cancel(&self, id: &str) -> bool {
        let map = self.downloads.lock().unwrap_or_else(|e| e.into_inner());
        map.get(id)
            .map(|c| c.store(true, Ordering::Relaxed))
            .is_some()
    }
}

/// Download registrato: si deregistra al drop (anche su errore o panic).
pub struct DownloadGuard<'a> {
    models: &'a Models,
    id: String,
    pub cancel: Arc<AtomicBool>,
}

impl Drop for DownloadGuard<'_> {
    fn drop(&mut self) {
        self.models
            .downloads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

pub fn spec(id: &str) -> Result<&'static ModelSpec, ModelError> {
    sv_models::find(id).ok_or_else(|| ModelError::Unknown(id.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_download_per_model_and_cancel() {
        let models = Models::new(std::env::temp_dir().join("sv-models-test-unused"));
        let guard = models.begin(sv_models::SILERO_VAD).unwrap();
        assert!(models.begin(sv_models::SILERO_VAD).is_none());
        assert!(models.begin(sv_models::CAMPLUS).is_some());
        assert!(
            models
                .list()
                .iter()
                .any(|m| m.spec.id == sv_models::SILERO_VAD && m.downloading)
        );

        assert!(models.cancel(sv_models::SILERO_VAD));
        assert!(guard.cancel.load(Ordering::Relaxed));
        drop(guard);
        assert!(!models.cancel(sv_models::SILERO_VAD));
        assert!(models.begin(sv_models::SILERO_VAD).is_some());
    }

    #[test]
    fn unknown_model_is_an_error() {
        assert!(matches!(spec("nope"), Err(ModelError::Unknown(_))));
    }
}
