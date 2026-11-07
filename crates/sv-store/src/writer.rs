//! Scrittura su SQLite fuori dal percorso critico della trascrizione.
//!
//! Il pump STT non tocca mai il disco: manda i dati a una coda limitata e un
//! thread dedicato li applica in transazione a blocchi di ~1 s. Se la coda è
//! piena (DB lentissimo) gli eventi vengono scartati con un warning: meglio
//! qualche frase mancante che una call che si blocca.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::StoreError;
use crate::schema;
use crate::types::{MeetingDraft, NewSegment};

/// Eventi in attesa oltre i quali si scarta (DB in difficoltà).
pub const QUEUE: usize = 2048;
/// Ogni quanto svuotare la coda su disco.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(1);
/// Dimensione oltre la quale si scrive subito, senza aspettare il tick.
const MAX_BATCH: usize = 512;

enum Command {
    Segment(NewSegment),
    Translation {
        session_id: u64,
        text: String,
        language: String,
        latency_ms: u64,
    },
    Relabel(Vec<(u64, Option<u8>)>),
    SpeakerNames(BTreeMap<u8, String>),
    Context {
        name: String,
        goal: String,
        notes: String,
    },
    Assist {
        kind: String,
        text: String,
        at_ms: u64,
    },
    Finish {
        elapsed_ms: Option<u64>,
        ack: SyncSender<Result<(), StoreError>>,
    },
}

/// Scrittore vivo per un meeting. Fermarlo con [`Writer::finish`] chiude il
/// meeting; il `Drop` fa lo stesso, al meglio, se la sessione viene distrutta
/// senza stop esplicito.
pub struct Writer {
    tx: SyncSender<Command>,
    thread: Option<JoinHandle<Result<(), StoreError>>>,
    meeting_id: i64,
}

/// Lato condivisibile (clone) per i task della sessione: non blocca mai.
#[derive(Clone)]
pub struct StoreHandle {
    tx: SyncSender<Command>,
}

impl Writer {
    /// Crea il meeting e avvia il thread di scrittura.
    pub fn start(path: impl AsRef<Path>, draft: MeetingDraft) -> Result<Self, StoreError> {
        let mut conn = schema::open_connection(path.as_ref())?;
        let started_at_ms = now_ms();
        let meeting_id = create_meeting(&mut conn, &draft, started_at_ms)?;
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        let thread = thread::Builder::new()
            .name("sv-store".into())
            .spawn(move || run(conn, rx, meeting_id, started_at_ms))?;
        Ok(Self {
            tx,
            thread: Some(thread),
            meeting_id,
        })
    }

    pub fn handle(&self) -> StoreHandle {
        StoreHandle {
            tx: self.tx.clone(),
        }
    }

    pub fn meeting_id(&self) -> i64 {
        self.meeting_id
    }

    /// Chiude il meeting dopo `elapsed_ms` dall'avvio, svuota la coda e
    /// attende il thread. Ritorna l'id del meeting.
    pub fn finish(mut self, elapsed_ms: u64) -> Result<i64, StoreError> {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        self.tx
            .send(Command::Finish {
                elapsed_ms: Some(elapsed_ms),
                ack: ack_tx,
            })
            .map_err(|_| StoreError::WriterClosed)?;
        ack_rx.recv_timeout(Duration::from_secs(10))??;
        self.join()?;
        Ok(self.meeting_id)
    }

    fn join(&mut self) -> Result<(), StoreError> {
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        match thread.join() {
            Ok(result) => result,
            Err(_) => Err(StoreError::WriterClosed),
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        if self.thread.is_none() {
            return;
        }
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        let _ = self.tx.send(Command::Finish {
            elapsed_ms: None,
            ack: ack_tx,
        });
        let _ = ack_rx.recv_timeout(Duration::from_secs(3));
        if let Err(e) = self.join() {
            tracing::warn!("chiusura del meeting salvato non riuscita: {e}");
        }
    }
}

impl StoreHandle {
    pub fn segment(&self, segment: NewSegment) {
        self.send(Command::Segment(segment));
    }

    pub fn translation(&self, session_id: u64, text: &str, language: &str, latency_ms: u64) {
        self.send(Command::Translation {
            session_id,
            text: text.to_owned(),
            language: language.to_owned(),
            latency_ms,
        });
    }

    /// Rietichettatura della diarization: anche frasi già salvate.
    pub fn relabel(&self, updates: Vec<(u64, Option<u8>)>) {
        if !updates.is_empty() {
            self.send(Command::Relabel(updates));
        }
    }

    /// Nomi correnti dei parlanti (sostituiscono i precedenti).
    pub fn speaker_names(&self, names: &BTreeMap<u8, String>) {
        self.send(Command::SpeakerNames(names.clone()));
    }

    pub fn context(&self, name: &str, goal: &str, notes: &str) {
        self.send(Command::Context {
            name: name.to_owned(),
            goal: goal.to_owned(),
            notes: notes.to_owned(),
        });
    }

    pub fn assist(&self, kind: &str, text: &str, at_ms: u64) {
        self.send(Command::Assist {
            kind: kind.to_owned(),
            text: text.to_owned(),
            at_ms,
        });
    }

    fn send(&self, command: Command) {
        match self.tx.try_send(command) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                tracing::warn!("coda di salvataggio piena: evento scartato");
            }
            Err(TrySendError::Disconnected(_)) => {
                tracing::warn!("salvataggio non attivo: evento scartato");
            }
        }
    }
}

fn run(
    mut conn: Connection,
    rx: Receiver<Command>,
    meeting_id: i64,
    started_at_ms: i64,
) -> Result<(), StoreError> {
    let mut pending = Vec::with_capacity(64);
    loop {
        match rx.recv_timeout(FLUSH_INTERVAL) {
            Ok(Command::Finish { elapsed_ms, ack }) => {
                let result = (|| {
                    flush(&mut conn, meeting_id, &mut pending)?;
                    finish_meeting(&conn, meeting_id, started_at_ms, elapsed_ms)
                })();
                if let Err(e) = &result {
                    tracing::warn!("chiusura del meeting non riuscita: {e}");
                }
                let _ = ack.send(result);
                return Ok(());
            }
            Ok(command) => {
                pending.push(command);
                if pending.len() >= MAX_BATCH {
                    flush(&mut conn, meeting_id, &mut pending)?;
                }
            }
            Err(RecvTimeoutError::Timeout) => flush(&mut conn, meeting_id, &mut pending)?,
            Err(RecvTimeoutError::Disconnected) => {
                flush(&mut conn, meeting_id, &mut pending)?;
                return Ok(());
            }
        }
    }
}

fn flush(
    conn: &mut Connection,
    meeting_id: i64,
    pending: &mut Vec<Command>,
) -> Result<(), StoreError> {
    if pending.is_empty() {
        return Ok(());
    }
    let batch = std::mem::take(pending);
    let tx = conn.transaction()?;
    let now = now_ms();
    for command in batch {
        apply(&tx, meeting_id, now, command)?;
    }
    tx.commit()?;
    Ok(())
}

fn apply(
    tx: &Transaction<'_>,
    meeting_id: i64,
    now_ms: i64,
    command: Command,
) -> Result<(), StoreError> {
    match command {
        Command::Segment(segment) => {
            tx.execute(
                "INSERT INTO segments
                     (meeting_id, session_id, channel, speaker, text, start_ms, end_ms,
                      language, confidence, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(meeting_id, session_id) DO UPDATE SET
                     channel = excluded.channel,
                     speaker = excluded.speaker,
                     text = excluded.text,
                     start_ms = excluded.start_ms,
                     end_ms = excluded.end_ms,
                     language = excluded.language,
                     confidence = excluded.confidence",
                params![
                    meeting_id,
                    to_i64(segment.session_id),
                    segment.channel,
                    segment.speaker.map(i64::from),
                    segment.text,
                    to_i64(segment.start_ms),
                    to_i64(segment.end_ms),
                    segment.language,
                    segment.confidence.map(f64::from),
                    now_ms,
                ],
            )?;
        }
        Command::Translation {
            session_id,
            text,
            language,
            latency_ms,
        } => {
            let segment_id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM segments WHERE meeting_id = ?1 AND session_id = ?2",
                    params![meeting_id, to_i64(session_id)],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(segment_id) = segment_id else {
                tracing::warn!(session_id, "traduzione senza frase salvata: scartata");
                return Ok(());
            };
            tx.execute(
                "INSERT INTO translations (segment_id, text, language, latency_ms, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(segment_id, language) DO UPDATE SET
                     text = excluded.text,
                     latency_ms = excluded.latency_ms,
                     created_at_ms = excluded.created_at_ms",
                params![segment_id, text, language, to_i64(latency_ms), now_ms],
            )?;
        }
        Command::Relabel(updates) => {
            for (session_id, speaker) in updates {
                tx.execute(
                    "UPDATE segments SET speaker = ?1
                     WHERE meeting_id = ?2 AND session_id = ?3",
                    params![speaker.map(i64::from), meeting_id, to_i64(session_id)],
                )?;
            }
        }
        Command::SpeakerNames(names) => {
            tx.execute(
                "DELETE FROM speakers WHERE meeting_id = ?1",
                params![meeting_id],
            )?;
            for (speaker, name) in names {
                if name.trim().is_empty() {
                    continue;
                }
                tx.execute(
                    "INSERT INTO speakers (meeting_id, speaker, name) VALUES (?1, ?2, ?3)",
                    params![meeting_id, i64::from(speaker), name],
                )?;
            }
        }
        Command::Context { name, goal, notes } => {
            tx.execute(
                "UPDATE meetings SET name = ?1, goal = ?2, notes = ?3 WHERE id = ?4",
                params![name, goal, notes, meeting_id],
            )?;
        }
        Command::Assist { kind, text, at_ms } => {
            tx.execute(
                "INSERT INTO assists (meeting_id, kind, text, at_ms, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![meeting_id, kind, text, to_i64(at_ms), now_ms],
            )?;
        }
        Command::Finish { .. } => unreachable!("Finish gestito dal loop"),
    }
    Ok(())
}

fn create_meeting(
    conn: &mut Connection,
    draft: &MeetingDraft,
    started_at_ms: i64,
) -> Result<i64, StoreError> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO meetings
             (name, goal, notes, started_at_ms, stt_engine, stt_model,
              my_language, their_language)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            draft.name,
            draft.goal,
            draft.notes,
            started_at_ms,
            draft.stt_engine,
            draft.stt_model,
            draft.my_language,
            draft.their_language,
        ],
    )?;
    let meeting_id = tx.last_insert_rowid();
    for (speaker, name) in &draft.speakers {
        if name.trim().is_empty() {
            continue;
        }
        tx.execute(
            "INSERT INTO speakers (meeting_id, speaker, name) VALUES (?1, ?2, ?3)",
            params![meeting_id, i64::from(*speaker), name],
        )?;
    }
    tx.commit()?;
    Ok(meeting_id)
}

fn finish_meeting(
    conn: &Connection,
    meeting_id: i64,
    started_at_ms: i64,
    elapsed_ms: Option<u64>,
) -> Result<(), StoreError> {
    let last_ms = match elapsed_ms {
        Some(ms) => to_i64(ms),
        None => conn.query_row(
            "SELECT MAX(
                        COALESCE((SELECT MAX(end_ms) FROM segments WHERE meeting_id = ?1), 0),
                        COALESCE((SELECT MAX(at_ms) FROM assists WHERE meeting_id = ?1), 0)
                    )",
            params![meeting_id],
            |row| row.get::<_, i64>(0),
        )?,
    };
    conn.execute(
        "UPDATE meetings SET ended_at_ms = ?1 WHERE id = ?2 AND ended_at_ms IS NULL",
        params![started_at_ms.saturating_add(last_ms), meeting_id],
    )?;
    Ok(())
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn draft() -> MeetingDraft {
        MeetingDraft {
            name: "Acme sync".into(),
            goal: "fissare la data".into(),
            stt_engine: "deepgram".into(),
            stt_model: "nova-3".into(),
            my_language: "it".into(),
            their_language: "multi".into(),
            speakers: BTreeMap::from([(0, "Marco".to_owned())]),
            ..MeetingDraft::default()
        }
    }

    fn segment(id: u64, speaker: Option<u8>) -> NewSegment {
        NewSegment {
            session_id: id,
            channel: "them".into(),
            speaker,
            text: format!("frase {id}"),
            start_ms: id * 1000,
            end_ms: id * 1000 + 900,
            language: Some("en".into()),
            confidence: Some(0.9),
        }
    }

    #[test]
    fn writes_and_reads_a_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sottovoce.db");
        let writer = Writer::start(&path, draft()).unwrap();
        let meeting_id = writer.meeting_id();
        let store = writer.handle();
        store.segment(segment(1, Some(0)));
        store.segment(segment(2, Some(1)));
        store.translation(1, "prima frase", "it", 120);
        store.relabel(vec![(2, Some(0))]);
        store.context("Acme sync 2", "chiudere", "note nuove");
        store.assist("say", "proposta", 1500);
        store.speaker_names(&BTreeMap::from([
            (0, "Marco".to_owned()),
            (1, "Anna".to_owned()),
        ]));
        assert_eq!(writer.finish(20_000).unwrap(), meeting_id);

        let store = Store::open(&path).unwrap();
        let details = store.details(meeting_id).unwrap();
        assert_eq!(details.meeting.name, "Acme sync 2");
        assert_eq!(details.meeting.goal, "chiudere");
        assert_eq!(details.meeting.notes, "note nuove");
        assert_eq!(details.meeting.stt_engine, "deepgram");
        assert_eq!(details.meeting.stt_model, "nova-3");
        assert_eq!(details.meeting.my_language, "it");
        assert_eq!(details.meeting.their_language, "multi");
        assert_eq!(
            details.meeting.ended_at_ms,
            Some(details.meeting.started_at_ms + 20_000)
        );
        assert_eq!(details.segments.len(), 2);
        assert_eq!(details.segments[0].text, "frase 1");
        assert_eq!(details.segments[0].speaker, Some(0));
        // Rietichettatura applicata alla frase già scritta.
        assert_eq!(details.segments[1].speaker, Some(0));
        assert_eq!(details.translations.len(), 1);
        assert_eq!(details.translations[0].session_id, 1);
        assert_eq!(details.translations[0].language, "it");
        assert_eq!(details.translations[0].latency_ms, 120);
        assert_eq!(
            details.speakers,
            [
                crate::types::SpeakerName {
                    speaker: 0,
                    name: "Marco".into()
                },
                crate::types::SpeakerName {
                    speaker: 1,
                    name: "Anna".into()
                },
            ]
        );
        assert_eq!(details.assists.len(), 1);
        assert_eq!(details.assists[0].kind, "say");
        assert_eq!(details.assists[0].at_ms, 1500);
    }

    #[test]
    fn drop_closes_the_meeting_at_the_last_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sottovoce.db");
        let writer = Writer::start(&path, draft()).unwrap();
        let meeting_id = writer.meeting_id();
        writer.handle().segment(segment(3, Some(0)));
        drop(writer);

        let store = Store::open(&path).unwrap();
        let meeting = store.meeting(meeting_id).unwrap();
        assert_eq!(meeting.ended_at_ms, Some(meeting.started_at_ms + 3_900));
    }
}
