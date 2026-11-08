//! Modelli su disco: `<dir>/<id>/<file>`. Download in `<file>.part` con resume,
//! verifica SHA-256 e rename atomico; un marker `.verified.json` registra
//! dimensione e data di modifica dei file verificati, così lo stato si
//! ricava senza ricalcolare gli hash a ogni avvio.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use reqwest::StatusCode;
use reqwest::header::RANGE;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::catalog::{ModelFile, ModelSpec};
use crate::error::ModelError;

const MARKER: &str = ".verified.json";
const PART_EXT: &str = "part";
/// Intervallo minimo fra due notifiche di avanzamento.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);
/// Tentativi per file su errore di rete (riprendono dal punto raggiunto).
const NETWORK_ATTEMPTS: u32 = 3;

/// Stato di un modello su disco.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ModelStatus {
    /// Nessun file presente.
    Missing,
    /// Download iniziato e non concluso (riprendibile).
    Partial { downloaded: u64 },
    /// Tutti i file presenti e verificati.
    Ready,
    /// File presenti ma alterati o non verificati: va riscaricato.
    Corrupt,
}

/// Avanzamento di un download (byte del modello intero).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Marker {
    files: Vec<FileStamp>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct FileStamp {
    name: String,
    size: u64,
    modified_ns: u128,
}

/// Cartella dei modelli locali.
#[derive(Debug, Clone)]
pub struct ModelStore {
    dir: PathBuf,
    client: reqwest::Client,
}

impl ModelStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .user_agent(concat!("sottovoce/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(20))
            .read_timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self {
            dir: dir.into(),
            client,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Cartella di un modello (esista o no).
    pub fn model_dir(&self, spec: &ModelSpec) -> PathBuf {
        self.dir.join(&spec.id)
    }

    /// Stato rapido: dimensioni e marker, senza ricalcolare gli hash.
    pub fn status(&self, spec: &ModelSpec) -> ModelStatus {
        let dir = self.model_dir(spec);
        let mut complete = 0;
        let mut downloaded = 0;
        let mut wrong_size = false;
        for f in &spec.files {
            match file_len(&dir.join(&f.name)) {
                Some(len) if len == f.size => {
                    complete += 1;
                    downloaded += len;
                }
                Some(_) => wrong_size = true,
                None => downloaded += file_len(&part_path(&dir, f)).unwrap_or(0).min(f.size),
            }
        }
        if wrong_size {
            return ModelStatus::Corrupt;
        }
        if complete == spec.files.len() {
            let ok =
                read_marker(&dir).is_some_and(|m| stamps(&dir, spec).is_some_and(|s| s == m.files));
            return if ok {
                ModelStatus::Ready
            } else {
                ModelStatus::Corrupt
            };
        }
        if downloaded == 0 {
            ModelStatus::Missing
        } else {
            ModelStatus::Partial { downloaded }
        }
    }

    /// Cartella del modello se pronto, altrimenti errore leggibile.
    pub fn ready_dir(&self, spec: &ModelSpec) -> Result<PathBuf, ModelError> {
        match self.status(spec) {
            ModelStatus::Ready => Ok(self.model_dir(spec)),
            status => Err(ModelError::NotReady {
                id: spec.id.clone(),
                status,
            }),
        }
    }

    /// Scarica (o completa) il modello. Riprende i `.part` esistenti, verifica
    /// ogni file e scrive il marker solo a download riuscito. `cancel` è
    /// controllato fra un blocco e l'altro: i `.part` restano per il resume.
    pub async fn download<F>(
        &self,
        spec: &ModelSpec,
        mut on_progress: F,
        cancel: &AtomicBool,
    ) -> Result<(), ModelError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        let dir = self.model_dir(spec);
        tokio::fs::create_dir_all(&dir).await?;
        remove_if_exists(&dir.join(MARKER))?;

        let total = spec.size();
        let mut progress = Throttle::new(total);
        let mut done = 0;
        for f in &spec.files {
            let target = dir.join(&f.name);
            if file_len(&target) == Some(f.size) && sha256_file(target.clone()).await? == f.sha256 {
                done += f.size;
                progress.report(done, true, &mut on_progress);
                continue;
            }
            remove_if_exists(&target)?;

            let part = part_path(&dir, f);
            self.fetch_with_retry(f, &part, done, &mut progress, &mut on_progress, cancel)
                .await?;
            if sha256_file(part.clone()).await? != f.sha256 {
                remove_if_exists(&part)?;
                return Err(ModelError::Checksum {
                    file: f.name.clone(),
                });
            }
            tokio::fs::rename(&part, &target).await?;
            done += f.size;
            progress.report(done, true, &mut on_progress);
        }

        let stamps = stamps(&dir, spec)
            .ok_or_else(|| io::Error::other("file scomparsi dopo il download"))?;
        let marker =
            serde_json::to_vec_pretty(&Marker { files: stamps }).map_err(io::Error::other)?;
        write_atomic(&dir.join(MARKER), &marker).await?;
        tracing::info!(model = %spec.id, bytes = total, "modello scaricato e verificato");
        Ok(())
    }

    /// Verifica completa (ricalcola gli hash). Aggiorna il marker se tutto
    /// torna e lo rimuove altrimenti.
    pub async fn verify(&self, spec: &ModelSpec) -> Result<ModelStatus, ModelError> {
        let dir = self.model_dir(spec);
        for f in &spec.files {
            let path = dir.join(&f.name);
            if file_len(&path) != Some(f.size) || sha256_file(path).await? != f.sha256 {
                remove_if_exists(&dir.join(MARKER))?;
                return Ok(self.status(spec));
            }
        }
        let stamps = stamps(&dir, spec)
            .ok_or_else(|| io::Error::other("file scomparsi durante la verifica"))?;
        let marker =
            serde_json::to_vec_pretty(&Marker { files: stamps }).map_err(io::Error::other)?;
        write_atomic(&dir.join(MARKER), &marker).await?;
        Ok(ModelStatus::Ready)
    }

    /// Elimina la cartella del modello (download parziali inclusi).
    pub async fn delete(&self, spec: &ModelSpec) -> Result<(), ModelError> {
        match tokio::fs::remove_dir_all(self.model_dir(spec)).await {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// Byte occupati su disco da un modello (anche parziale).
    pub fn disk_usage(&self, spec: &ModelSpec) -> u64 {
        std::fs::read_dir(self.model_dir(spec))
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter_map(|e| e.metadata().ok())
                    .filter(|m| m.is_file())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0)
    }

    async fn fetch_with_retry<F>(
        &self,
        f: &ModelFile,
        part: &Path,
        base: u64,
        progress: &mut Throttle,
        on_progress: &mut F,
        cancel: &AtomicBool,
    ) -> Result<(), ModelError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self
                .fetch(f, part, base, progress, on_progress, cancel)
                .await
            {
                Err(ModelError::Network(e)) if attempt < NETWORK_ATTEMPTS => {
                    tracing::warn!(file = %f.name, attempt, "download interrotto, riprendo: {e}");
                    tokio::time::sleep(Duration::from_secs(u64::from(attempt))).await;
                }
                other => return other,
            }
        }
    }

    /// Scarica un file in `part`, riprendendo dai byte già presenti.
    async fn fetch<F>(
        &self,
        f: &ModelFile,
        part: &Path,
        base: u64,
        progress: &mut Throttle,
        on_progress: &mut F,
        cancel: &AtomicBool,
    ) -> Result<(), ModelError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        let mut have = file_len(part).unwrap_or(0);
        if have > f.size {
            remove_if_exists(part)?;
            have = 0;
        }
        if have == f.size {
            return Ok(());
        }

        let mut req = self.client.get(&f.url);
        if have > 0 {
            req = req.header(RANGE, format!("bytes={have}-"));
        }
        let mut resp = req.send().await.map_err(network)?;
        let append = match resp.status() {
            StatusCode::PARTIAL_CONTENT => true,
            StatusCode::OK => false,
            StatusCode::RANGE_NOT_SATISFIABLE => {
                // Il `.part` non corrisponde più al file remoto: si riparte.
                remove_if_exists(part)?;
                return Err(ModelError::Network(
                    "range non valido, download da ricominciare".into(),
                ));
            }
            status => {
                return Err(ModelError::Http {
                    status: status.as_u16(),
                });
            }
        };
        if !append {
            have = 0;
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(part)
            .await?;
        while let Some(chunk) = resp.chunk().await.map_err(network)? {
            if cancel.load(Ordering::Relaxed) {
                file.flush().await?;
                return Err(ModelError::Cancelled);
            }
            file.write_all(&chunk).await?;
            have += chunk.len() as u64;
            if have > f.size {
                drop(file);
                remove_if_exists(part)?;
                return Err(ModelError::Checksum {
                    file: f.name.clone(),
                });
            }
            progress.report(base + have, false, on_progress);
        }
        file.flush().await?;
        if have < f.size {
            return Err(ModelError::Network(format!(
                "{}: connessione chiusa a {have} di {} byte",
                f.name, f.size
            )));
        }
        Ok(())
    }
}

struct Throttle {
    total: u64,
    last: Option<Instant>,
}

impl Throttle {
    fn new(total: u64) -> Self {
        Self { total, last: None }
    }

    fn report<F: FnMut(DownloadProgress)>(&mut self, downloaded: u64, force: bool, f: &mut F) {
        let now = Instant::now();
        if force || self.last.is_none_or(|t| now - t >= PROGRESS_EVERY) {
            self.last = Some(now);
            f(DownloadProgress {
                downloaded,
                total: self.total,
            });
        }
    }
}

fn network(e: reqwest::Error) -> ModelError {
    ModelError::Network(e.to_string())
}

fn part_path(dir: &Path, f: &ModelFile) -> PathBuf {
    dir.join(format!("{}.{PART_EXT}", f.name))
}

fn file_len(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn read_marker(dir: &Path) -> Option<Marker> {
    serde_json::from_slice(&std::fs::read(dir.join(MARKER)).ok()?).ok()
}

fn stamps(dir: &Path, spec: &ModelSpec) -> Option<Vec<FileStamp>> {
    spec.files
        .iter()
        .map(|f| {
            let meta = std::fs::metadata(dir.join(&f.name)).ok()?;
            let modified_ns = meta
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_nanos();
            Some(FileStamp {
                name: f.name.clone(),
                size: meta.len(),
                modified_ns,
            })
        })
        .collect()
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await
}

/// SHA-256 esadecimale di un file, calcolato fuori dal runtime async.
async fn sha256_file(path: PathBuf) -> io::Result<String> {
    tokio::task::spawn_blocking(move || {
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect())
    })
    .await
    .map_err(io::Error::other)?
}
