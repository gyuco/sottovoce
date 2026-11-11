//! Ricerca semantica nello storico.
//!
//! Un thread dedicato (`sv-index`, QoS "utility" su macOS) possiede il
//! modello di embedding e serve due code:
//! - query di ricerca (precedenza assoluta, anche a metà di un meeting);
//! - meeting da indicizzare, a lotti di [`BATCH`] blocchi.
//!
//! Il modello si carica al primo lavoro e si scarica dopo [`IDLE_UNLOAD`] di
//! inattività (circa 500 MB di RAM). I vettori finiscono in
//! `app_data_dir/index/<meeting_id>.vec` (vedi `sv-search`): mai nel DB.
//!
//! Quando: a fine sessione, dopo il riepilogo, alla rinomina, all'avvio
//! (backfill dei meeting senza indice o con indice vecchio) e quando il
//! modello finisce di scaricarsi o la ricerca semantica viene riattivata.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use sv_embed::Embedder;
use sv_models::ModelStore;
use sv_search::{ChunkRef, SemanticIndex};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use crate::settings::SettingsStore;

/// Evento con lo stato dell'indicizzazione ([`IndexStatus`]).
pub const STATUS_EVENT: &str = "search:status";
/// Blocchi per esecuzione del modello: tra un lotto e l'altro si servono le
/// query in attesa.
const BATCH: usize = 8;
/// Il modello resta in memoria per questo tempo dopo l'ultimo lavoro.
const IDLE_UNLOAD: Duration = Duration::from_secs(5 * 60);
/// Thread di calcolo di ONNX Runtime: pochi, è lavoro in background.
const THREADS: usize = 2;
/// Passaggi semantici considerati per query.
const PASSAGES: usize = 40;
/// Passaggi tenuti se distano al massimo così dal migliore (coseno).
const MARGIN: f32 = 0.05;
/// Passaggi mostrati per meeting.
const PER_MEETING: usize = 3;
/// Meeting nei risultati.
const MAX_RESULTS: usize = 20;
/// Tempo massimo per l'embedding della query (modello da caricare incluso).
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);

/// Stato mostrato in Impostazioni e nello Storico.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    /// Modello scaricato e verificato.
    pub model_ready: bool,
    /// Setting `semanticSearch`.
    pub enabled: bool,
    /// Meeting con un indice aggiornato (ultimo conteggio noto).
    pub indexed: u32,
    /// Meeting nello storico.
    pub total: u32,
    /// Meeting in coda o in lavorazione.
    pub pending: u32,
    /// Ultimo errore di indicizzazione, se c'è.
    pub error: Option<String>,
}

type QueryReply = oneshot::Sender<Result<Vec<f32>>>;

#[derive(Default)]
struct Queue {
    queries: VecDeque<(String, QueryReply)>,
    meetings: VecDeque<i64>,
    /// Controllare tutti i meeting (avvio, modello scaricato, riattivazione).
    backfill: bool,
    /// Scaricare il modello (eliminato o cambiato).
    reload: bool,
}

struct Inner {
    app: AppHandle,
    db_path: PathBuf,
    models: ModelStore,
    index: SemanticIndex,
    queue: Mutex<Queue>,
    wake: Condvar,
    status: Mutex<IndexStatus>,
}

/// Stato Tauri della ricerca semantica.
pub struct Search {
    inner: Arc<Inner>,
}

impl Search {
    /// Copia che condivide lo stesso servizio (per i thread bloccanti).
    pub fn clone_handle(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    /// Crea il servizio e avvia il thread; il primo backfill parte subito.
    pub fn start(app: AppHandle, data_dir: PathBuf, db_path: PathBuf) -> Self {
        let inner = Arc::new(Inner {
            app,
            db_path,
            models: ModelStore::new(data_dir.join("models")),
            index: SemanticIndex::new(
                data_dir.join("index"),
                sv_models::EMBEDDING_GEMMA_2,
                sv_embed::DIM,
            ),
            queue: Mutex::new(Queue {
                backfill: true,
                ..Queue::default()
            }),
            wake: Condvar::new(),
            status: Mutex::default(),
        });
        let worker = inner.clone();
        if let Err(e) = std::thread::Builder::new()
            .name("sv-index".into())
            .spawn(move || worker.run())
        {
            tracing::warn!("thread di indicizzazione non avviato: {e}");
        }
        Self { inner }
    }

    /// Mette in coda l'indicizzazione di un meeting.
    pub fn enqueue(&self, meeting_id: i64) {
        let mut q = self.inner.queue();
        if !q.meetings.contains(&meeting_id) {
            q.meetings.push_back(meeting_id);
        }
        drop(q);
        self.inner.wake.notify_one();
    }

    /// Ricontrolla tutti i meeting (es. dopo il download del modello).
    pub fn backfill(&self) {
        self.inner.queue().backfill = true;
        self.inner.wake.notify_one();
    }

    /// Il modello è stato eliminato o riscaricato: libera quello caricato e
    /// ricontrolla gli indici.
    pub fn model_changed(&self) {
        let mut q = self.inner.queue();
        q.reload = true;
        q.backfill = true;
        drop(q);
        self.inner.wake.notify_one();
    }

    /// Rimuove l'indice di un meeting eliminato.
    pub fn remove(&self, meeting_id: i64) {
        self.inner.queue().meetings.retain(|&id| id != meeting_id);
        if let Err(e) = self.inner.index.remove(meeting_id) {
            tracing::warn!(meeting = meeting_id, "indice non eliminato: {e}");
        }
        self.inner.refresh_status(None);
    }

    pub fn status(&self) -> IndexStatus {
        self.inner.refresh_status(None)
    }

    /// Ricerca ibrida: semantica (se il modello è pronto e la funzione è
    /// attiva) fusa con quella per parole. Senza modello resta la seconda.
    pub async fn search(&self, query: String) -> Result<Vec<SearchResult>> {
        let query = query.trim().to_owned();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let semantic = self.semantic_passages(&query).await;
        let inner = self.inner.clone();
        tauri::async_runtime::spawn_blocking(move || inner.collect(&query, semantic))
            .await
            .map_err(|e| anyhow!("{e}"))?
    }

    async fn semantic_passages(&self, query: &str) -> Vec<sv_search::PassageHit> {
        self.semantic_where(query, PASSAGES, MARGIN, |_| true)
            .await
            .unwrap_or_default()
    }

    /// Passaggi semantici tra i meeting accettati da `keep`. `None` se la
    /// ricerca per significato non è disponibile (spenta, modello assente,
    /// embedding non riuscito): resta solo quella per parole.
    async fn semantic_where(
        &self,
        query: &str,
        limit: usize,
        margin: f32,
        keep: impl Fn(i64) -> bool,
    ) -> Option<Vec<sv_search::PassageHit>> {
        let status = self.inner.refresh_status(None);
        if !status.enabled || !status.model_ready {
            return None;
        }
        if self.inner.index.indexed().is_empty() {
            return Some(Vec::new());
        }
        let (tx, rx) = oneshot::channel();
        self.inner.queue().queries.push_back((query.to_owned(), tx));
        self.inner.wake.notify_one();
        let vector = match tokio::time::timeout(QUERY_TIMEOUT, rx).await {
            Ok(Ok(Ok(v))) => v,
            Ok(Ok(Err(e))) => {
                tracing::warn!("embedding della query non riuscito: {e:#}");
                return None;
            }
            Ok(Err(_)) | Err(_) => {
                tracing::warn!("embedding della query scaduto");
                return None;
            }
        };
        Some(self.inner.index.search_where(&vector, limit, margin, keep))
    }

    /// Recupero per le domande sullo storico: passaggi semantici e per
    /// parole (a livello di blocco) fusi con RRF, al massimo
    /// [`sv_search::PER_MEETING`] per meeting, più riepilogo e action item
    /// dei meeting trovati, in un contesto numerato entro il budget.
    pub async fn retrieve(
        &self,
        query: String,
        filter: HistoryFilter,
        utc_offset_min: i32,
    ) -> Result<Retrieved> {
        let db_path = self.inner.db_path.clone();
        let allowed: Vec<i64> = tauri::async_runtime::spawn_blocking(move || {
            let meetings = sv_store::Store::open(&db_path)?.meetings()?;
            anyhow::Ok(
                meetings
                    .iter()
                    .filter(|m| filter.keeps(m))
                    .map(|m| m.id)
                    .collect(),
            )
        })
        .await
        .map_err(|e| anyhow!("{e}"))??;
        if allowed.is_empty() {
            return Ok(Retrieved::default());
        }
        let semantic = {
            let allowed = allowed.clone();
            self.semantic_where(&query, RAG_PASSAGES, RAG_MARGIN, move |id| {
                allowed.contains(&id)
            })
            .await
        };
        let inner = self.inner.clone();
        tauri::async_runtime::spawn_blocking(move || {
            inner.retrieve(&query, &allowed, semantic, utc_offset_min)
        })
        .await
        .map_err(|e| anyhow!("{e}"))?
    }
}

/// Passaggi semantici considerati per una domanda sullo storico.
const RAG_PASSAGES: usize = 60;
/// Margine più largo della ricerca: le domande sono frasi lunghe e il
/// passaggio giusto può stare un po' sotto al migliore.
const RAG_MARGIN: f32 = 0.08;
/// Passaggi per parole considerati per una domanda.
const RAG_LEXICAL: usize = 40;

/// Filtri opzionali delle domande sullo storico.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryFilter {
    /// Meeting iniziati da questo istante (ms epoch, incluso).
    pub from_ms: Option<i64>,
    /// Meeting iniziati prima di questo istante (ms epoch, escluso).
    pub to_ms: Option<i64>,
    /// Solo questo meeting.
    pub meeting_id: Option<i64>,
}

impl HistoryFilter {
    fn keeps(&self, m: &sv_store::Meeting) -> bool {
        self.meeting_id.is_none_or(|id| id == m.id)
            && self.from_ms.is_none_or(|from| m.started_at_ms >= from)
            && self.to_ms.is_none_or(|to| m.started_at_ms < to)
    }
}

/// Contesto numerato per il modello.
#[derive(Debug, Default)]
pub struct Retrieved {
    pub context: sv_search::Context,
    /// La ricerca per significato ha partecipato (altrimenti solo parole).
    pub semantic: bool,
}

/// Passaggio di un risultato, con il testo riletto dal meeting.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Passage {
    /// `transcript`, `block`, `overview`, `keyPoints`, `actionItems`.
    pub kind: &'static str,
    /// Prima frase del passaggio (solo trascrizione): per scorrere al punto.
    pub segment_id: Option<i64>,
    pub start_ms: u64,
    pub text: String,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(flatten)]
    pub meeting: sv_store::MeetingSummary,
    pub score: f64,
    /// Trovato anche dalla ricerca per parole.
    pub lexical: bool,
    pub passages: Vec<Passage>,
}

pub fn kind_name(source: ChunkRef) -> (&'static str, Option<i64>) {
    match source {
        ChunkRef::Transcript { first, .. } => ("transcript", Some(first)),
        ChunkRef::Block { .. } => ("block", None),
        ChunkRef::Overview => ("overview", None),
        ChunkRef::KeyPoints { .. } => ("keyPoints", None),
        ChunkRef::ActionItems => ("actionItems", None),
    }
}

impl Inner {
    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn enabled(&self) -> bool {
        self.app
            .try_state::<SettingsStore>()
            .is_none_or(|s| s.get().semantic_search)
    }

    fn model_dir(&self) -> Option<PathBuf> {
        let spec = sv_models::find(sv_models::EMBEDDING_GEMMA_2)?;
        self.models.ready_dir(spec).ok()
    }

    /// Aggiorna lo stato (con `error` se dato) e lo invia alla UI.
    fn refresh_status(&self, error: Option<Option<String>>) -> IndexStatus {
        let total = sv_store::Store::open(&self.db_path)
            .and_then(|s| s.meetings())
            .map(|m| m.iter().map(|m| m.id).collect::<Vec<_>>())
            .unwrap_or_default();
        let indexed = self
            .index
            .indexed()
            .iter()
            .filter(|id| total.contains(id))
            .count();
        let pending = self.queue().meetings.len();
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        let next = IndexStatus {
            model_ready: self.model_dir().is_some(),
            enabled: self.enabled(),
            indexed: u32::try_from(indexed).unwrap_or(u32::MAX),
            total: u32::try_from(total.len()).unwrap_or(u32::MAX),
            pending: u32::try_from(pending).unwrap_or(u32::MAX),
            error: error.unwrap_or_else(|| status.error.clone()),
        };
        if *status != next {
            *status = next.clone();
            if let Err(e) = self.app.emit(STATUS_EVENT, &next) {
                tracing::debug!("stato indicizzazione non inviato: {e}");
            }
        }
        next
    }

    fn run(self: Arc<Self>) {
        set_background_qos();
        let mut embedder: Option<Embedder> = None;
        let mut last_used = Instant::now();
        loop {
            // Attende lavoro; scarica il modello se resta inattivo.
            let job = {
                let mut q = self.queue();
                loop {
                    if q.reload {
                        q.reload = false;
                        embedder = None;
                    }
                    if let Some(job) = next_job(&mut q, self.enabled()) {
                        break job;
                    }
                    let wait = IDLE_UNLOAD.saturating_sub(last_used.elapsed());
                    if embedder.is_some() && wait.is_zero() {
                        tracing::info!("modello di embedding scaricato dalla memoria");
                        embedder = None;
                    }
                    let timeout = if embedder.is_some() {
                        wait
                    } else {
                        Duration::from_secs(3600)
                    };
                    q = self
                        .wake
                        .wait_timeout(q, timeout)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            };
            match job {
                Job::Query(text, reply) => {
                    let result = self
                        .ensure(&mut embedder)
                        .and_then(|e| e.embed_query(&text).map_err(|e| anyhow!("{e}")));
                    let _ = reply.send(result);
                }
                Job::Backfill => self.backfill(),
                Job::Meeting(id) => {
                    let error = match self.index_meeting(id, &mut embedder) {
                        Ok(()) => None,
                        Err(e) => {
                            tracing::warn!(meeting = id, "indicizzazione non riuscita: {e:#}");
                            Some(format!("{e:#}"))
                        }
                    };
                    self.refresh_status(Some(error));
                }
            }
            last_used = Instant::now();
        }
    }

    fn ensure<'a>(&self, embedder: &'a mut Option<Embedder>) -> Result<&'a mut Embedder> {
        if embedder.is_none() {
            let dir = self
                .model_dir()
                .context("modello di ricerca semantica non scaricato")?;
            let started = Instant::now();
            *embedder = Some(Embedder::open(&dir, THREADS)?);
            tracing::info!(
                ms = started.elapsed().as_millis() as u64,
                "modello di embedding caricato"
            );
        }
        Ok(embedder.as_mut().expect("appena caricato"))
    }

    /// Mette in coda i meeting senza indice o con indice vecchio, ed elimina
    /// gli indici dei meeting che non esistono più.
    fn backfill(&self) {
        if self.model_dir().is_none() || !self.enabled() {
            self.refresh_status(None);
            return;
        }
        let store = match sv_store::Store::open(&self.db_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("backfill dell'indice: database non disponibile: {e}");
                return;
            }
        };
        let meetings = store.meetings().unwrap_or_default();
        let ids: Vec<i64> = meetings.iter().map(|m| m.id).collect();
        self.index.retain(&ids);
        let mut stale = Vec::new();
        for m in &meetings {
            // Il meeting in corso cambia di continuo: si indicizza allo stop.
            if m.ended_at_ms.is_none() {
                continue;
            }
            match store.details(m.id) {
                Ok(details) => {
                    let prepared = sv_search::prepare(&details);
                    if !self.index.is_fresh(m.id, prepared.fingerprint)
                        && !prepared.chunks.is_empty()
                    {
                        stale.push(m.id);
                    }
                }
                Err(e) => tracing::warn!(meeting = m.id, "meeting illeggibile: {e}"),
            }
        }
        if !stale.is_empty() {
            tracing::info!(meetings = stale.len(), "meeting da indicizzare");
        }
        let mut q = self.queue();
        for id in stale {
            if !q.meetings.contains(&id) {
                q.meetings.push_back(id);
            }
        }
        drop(q);
        self.refresh_status(None);
    }

    fn index_meeting(&self, id: i64, embedder: &mut Option<Embedder>) -> Result<()> {
        let store = sv_store::Store::open(&self.db_path)?;
        let details = match store.details(id) {
            Ok(d) => d,
            Err(sv_store::StoreError::MeetingNotFound(_)) => {
                self.index.remove(id)?;
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let prepared = sv_search::prepare(&details);
        if self.index.is_fresh(id, prepared.fingerprint) {
            return Ok(());
        }
        let started = Instant::now();
        let mut vectors = Vec::with_capacity(prepared.prompts.len());
        for batch in prepared.prompts.chunks(BATCH) {
            self.serve_queries(embedder);
            let e = self.ensure(embedder)?;
            vectors.extend(e.embed_documents(batch)?);
        }
        self.index.store(id, &prepared, &vectors)?;
        tracing::info!(
            meeting = id,
            chunks = prepared.chunks.len(),
            ms = started.elapsed().as_millis() as u64,
            "meeting indicizzato"
        );
        Ok(())
    }

    /// Risponde alle query arrivate durante un'indicizzazione.
    fn serve_queries(&self, embedder: &mut Option<Embedder>) {
        loop {
            let Some((text, reply)) = self.queue().queries.pop_front() else {
                return;
            };
            let result = self
                .ensure(embedder)
                .and_then(|e| e.embed_query(&text).map_err(|e| anyhow!("{e}")));
            let _ = reply.send(result);
        }
    }

    fn retrieve(
        &self,
        query: &str,
        allowed: &[i64],
        semantic: Option<Vec<sv_search::PassageHit>>,
        utc_offset_min: i32,
    ) -> Result<Retrieved> {
        let store = sv_store::Store::open(&self.db_path)?;
        let mut meetings = HashMap::new();
        let mut chunks = Vec::with_capacity(allowed.len());
        for &id in allowed {
            match store.details(id) {
                Ok(details) => {
                    chunks.push((id, sv_search::chunks(&details)));
                    meetings.insert(id, details);
                }
                // Eliminato nel frattempo.
                Err(sv_store::StoreError::MeetingNotFound(_)) => {}
                Err(e) => return Err(e.into()),
            }
        }
        let lexical = sv_search::lexical_hits(&chunks, &sv_search::query_terms(query), RAG_LEXICAL);
        let used = semantic.is_some();
        let candidates = sv_search::fuse_passages(
            &semantic.unwrap_or_default(),
            &lexical,
            sv_search::PER_MEETING,
            sv_search::MAX_PASSAGES,
        );
        let context = sv_search::build_context(
            &candidates,
            &meetings,
            sv_search::CONTEXT_BUDGET,
            utc_offset_min,
        );
        tracing::info!(
            semantic = used,
            lexical = lexical.len(),
            sources = context.sources.len(),
            chars = context.text.chars().count(),
            "contesto per la domanda sullo storico"
        );
        Ok(Retrieved {
            context,
            semantic: used,
        })
    }

    /// Unisce risultati semantici e per parole e rilegge i testi.
    fn collect(
        &self,
        query: &str,
        semantic: Vec<sv_search::PassageHit>,
    ) -> Result<Vec<SearchResult>> {
        let store = sv_store::Store::open(&self.db_path)?;
        let lexical: Vec<i64> = store
            .list_meetings(0, 100, Some(query))?
            .meetings
            .iter()
            .map(|m| m.meeting.id)
            .collect();
        let hits = sv_search::fuse(&semantic, &lexical, PER_MEETING);
        let mut out = Vec::new();
        for hit in hits.into_iter().take(MAX_RESULTS) {
            let details = match store.details(hit.meeting_id) {
                Ok(d) => d,
                // Eliminato nel frattempo.
                Err(sv_store::StoreError::MeetingNotFound(_)) => continue,
                Err(e) => return Err(e.into()),
            };
            let passages = hit
                .passages
                .iter()
                .filter_map(|p| {
                    let text = sv_search::passage_text(&details, p.source)?;
                    let (kind, segment_id) = kind_name(p.source);
                    Some(Passage {
                        kind,
                        segment_id,
                        start_ms: p.start_ms,
                        text,
                        score: p.score,
                    })
                })
                .collect();
            out.push(SearchResult {
                meeting: sv_store::MeetingSummary {
                    segment_count: u32::try_from(details.segments.len()).unwrap_or(u32::MAX),
                    meeting: details.meeting,
                },
                score: hit.score,
                lexical: hit.lexical,
                passages,
            });
        }
        Ok(out)
    }
}

enum Job {
    Query(String, QueryReply),
    Backfill,
    Meeting(i64),
}

/// Prossimo lavoro: prima le query, poi il backfill, poi i meeting. Con la
/// ricerca semantica spenta le code dei meeting si svuotano senza lavoro.
fn next_job(q: &mut Queue, enabled: bool) -> Option<Job> {
    if let Some((text, reply)) = q.queries.pop_front() {
        return Some(Job::Query(text, reply));
    }
    if !enabled {
        q.backfill = false;
        q.meetings.clear();
        return None;
    }
    if q.backfill {
        q.backfill = false;
        return Some(Job::Backfill);
    }
    q.meetings.pop_front().map(Job::Meeting)
}

/// Priorità "utility" su macOS: l'indicizzazione non deve rubare CPU alla
/// trascrizione. I thread di ONNX Runtime creati da qui la ereditano.
fn set_background_qos() {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: imposta solo la classe QoS del thread corrente.
        let rc =
            unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0) };
        if rc != 0 {
            tracing::debug!(rc, "QoS del thread di indicizzazione non impostata");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_come_first_and_disabled_drains_meetings() {
        let mut q = Queue {
            meetings: VecDeque::from([1, 2]),
            backfill: true,
            ..Queue::default()
        };
        let (tx, _rx) = oneshot::channel();
        q.queries.push_back(("x".into(), tx));
        assert!(matches!(next_job(&mut q, true), Some(Job::Query(..))));
        assert!(matches!(next_job(&mut q, true), Some(Job::Backfill)));
        assert!(matches!(next_job(&mut q, true), Some(Job::Meeting(1))));
        assert!(next_job(&mut q, false).is_none());
        assert!(q.meetings.is_empty());
        // Le query passano anche a funzione spenta (rispondono con errore o
        // vengono servite): non restano appese.
        let (tx, _rx) = oneshot::channel();
        q.queries.push_back(("y".into(), tx));
        assert!(matches!(next_job(&mut q, false), Some(Job::Query(..))));
    }

    #[test]
    fn kind_names_match_the_frontend() {
        assert_eq!(
            kind_name(ChunkRef::Transcript { first: 4, last: 9 }),
            ("transcript", Some(4))
        );
        assert_eq!(kind_name(ChunkRef::KeyPoints { n: 1 }), ("keyPoints", None));
    }

    #[test]
    fn history_filter_dates_and_meeting() {
        let m = |id, started| sv_store::Meeting {
            id,
            name: String::new(),
            goal: String::new(),
            notes: String::new(),
            started_at_ms: started,
            ended_at_ms: None,
            stt_engine: String::new(),
            stt_model: String::new(),
            my_language: String::new(),
            their_language: String::new(),
        };
        assert!(HistoryFilter::default().keeps(&m(1, 0)));
        let range = HistoryFilter {
            from_ms: Some(100),
            to_ms: Some(200),
            meeting_id: None,
        };
        assert!(!range.keeps(&m(1, 99)));
        assert!(range.keeps(&m(1, 100)));
        assert!(!range.keeps(&m(1, 200)));
        let one = HistoryFilter {
            meeting_id: Some(2),
            ..HistoryFilter::default()
        };
        assert!(one.keeps(&m(2, 0)) && !one.keeps(&m(3, 0)));
    }
}
