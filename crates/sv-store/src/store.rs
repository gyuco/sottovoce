//! Lettura del database dei meeting.
//!
//! [`Store::open`] applica le migrazioni e chiude i meeting rimasti aperti
//! (crash o chiusura forzata dell'app). Le query sono sincrone e pensate per
//! l'avvio, lo storico e i futuri comandi Tauri.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::StoreError;
use crate::schema;
use crate::transcript;
use crate::types::{
    ActionItem, Assist, CompressionBlock, Meeting, MeetingDetails, MeetingPage, MeetingSummary,
    NewCompressionBlock, NewSummary, Segment, SpeakerName, Summary, Translation,
};

pub struct Store {
    conn: Mutex<Connection>,
    /// Cartella del database: i file di trascrizione sono relativi a questa.
    dir: PathBuf,
}

impl Store {
    /// Apre (creando se serve) `sottovoce.db` e applica le migrazioni.
    ///
    /// La chiusura dei meeting rimasti appesi non è automatica: l'app la fa
    /// una volta all'avvio con [`Store::recover_open_meetings`], così una
    /// lettura durante una call non tocca il meeting in corso.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
        let conn = schema::open_connection(path)?;
        Ok(Self {
            conn: Mutex::new(conn),
            dir: schema::base_dir(path),
        })
    }

    /// Chiude i meeting rimasti aperti (chiusura anomala dell'app):
    /// `fine = avvio + ultimo evento noto`. Ritorna quanti meeting ha chiuso.
    pub fn recover_open_meetings(&self) -> Result<usize, StoreError> {
        let mut conn = self.conn();
        schema::close_open_meetings(&mut conn)
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Percorso assoluto del file di trascrizione a partire dal riferimento
    /// salvato in `meetings.transcript_file`. Se il riferimento manca (righe
    /// create prima della migrazione 4 o DB nuovo) si usa il percorso standard.
    fn resolve(&self, file: &str, meeting_id: i64) -> PathBuf {
        if file.is_empty() {
            self.dir.join(transcript::relative_path(meeting_id))
        } else {
            self.dir.join(file)
        }
    }

    /// Meeting dal più recente.
    pub fn meetings(&self) -> Result<Vec<Meeting>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, goal, notes, started_at_ms, ended_at_ms,
                    stt_engine, stt_model, my_language, their_language
             FROM meetings ORDER BY started_at_ms DESC, id DESC",
        )?;
        let rows = stmt.query_map([], meeting_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Pagina dello storico. Con `search` filtra i meeting il cui file di
    /// trascrizione contiene tutte le parole (ricerca per prefisso, senza
    /// distinzione di maiuscole/accenti). La ricerca legge i file, non il DB:
    /// il testo delle frasi non è più nelle tabelle.
    pub fn list_meetings(
        &self,
        offset: u32,
        limit: u32,
        search: Option<&str>,
    ) -> Result<MeetingPage, StoreError> {
        let query = search.map(str::trim).filter(|q| !q.is_empty());
        let Some(query) = query else {
            return self.all_meetings(offset, limit);
        };
        let terms = search_terms(query);
        if terms.is_empty() {
            return Ok(MeetingPage {
                meetings: Vec::new(),
                total: 0,
            });
        }
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.name, m.goal, m.notes, m.started_at_ms, m.ended_at_ms,
                    m.stt_engine, m.stt_model, m.my_language, m.their_language,
                    (SELECT COUNT(*) FROM segments x WHERE x.meeting_id = m.id),
                    m.transcript_file
             FROM meetings m
             ORDER BY m.started_at_ms DESC, m.id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((meeting_summary_from_row(row)?, row.get::<_, String>(11)?))
        })?;
        let all = rows.collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        drop(conn);

        let mut matched = Vec::new();
        for (summary, file) in all {
            let path = self.resolve(&file, summary.meeting.id);
            if lines_match(&transcript::read_lines(&path)?, &terms) {
                matched.push(summary);
            }
        }
        let total = u32::try_from(matched.len()).unwrap_or(u32::MAX);
        Ok(MeetingPage {
            meetings: matched
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .collect(),
            total,
        })
    }

    fn all_meetings(&self, offset: u32, limit: u32) -> Result<MeetingPage, StoreError> {
        let conn = self.conn();
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM meetings", [], |row| row.get(0))?;
        let mut stmt = conn.prepare(
            "SELECT m.id, m.name, m.goal, m.notes, m.started_at_ms, m.ended_at_ms,
                    m.stt_engine, m.stt_model, m.my_language, m.their_language,
                    (SELECT COUNT(*) FROM segments x WHERE x.meeting_id = m.id)
             FROM meetings m
             ORDER BY m.started_at_ms DESC, m.id DESC
             LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(
            params![i64::from(limit), i64::from(offset)],
            meeting_summary_from_row,
        )?;
        Ok(MeetingPage {
            meetings: rows.collect::<Result<Vec<_>, _>>()?,
            total: u32::try_from(total).unwrap_or(u32::MAX),
        })
    }

    /// Elimina meeting, frasi, traduzioni, parlanti e assist in cascata, più
    /// il file di trascrizione associato.
    pub fn delete_meeting(&self, id: i64) -> Result<(), StoreError> {
        let file: Option<String> = {
            let conn = self.conn();
            conn.query_row(
                "SELECT transcript_file FROM meetings WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?
        };
        let conn = self.conn();
        let deleted = conn.execute("DELETE FROM meetings WHERE id = ?1", params![id])?;
        drop(conn);
        if deleted == 0 {
            return Err(StoreError::MeetingNotFound(id));
        }
        if let Some(file) = file {
            let path = self.resolve(&file, id);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    tracing::warn!(path = %path.display(), "file di trascrizione non eliminato: {e}")
                }
            }
        }
        Ok(())
    }

    /// Cambia il nome mostrato nello storico.
    pub fn rename_meeting(&self, id: i64, name: &str) -> Result<Meeting, StoreError> {
        let name: String = name.trim().chars().take(200).collect();
        if name.is_empty() {
            return Err(StoreError::EmptyName);
        }
        let conn = self.conn();
        let updated = conn.execute(
            "UPDATE meetings SET name = ?1 WHERE id = ?2",
            params![name, id],
        )?;
        if updated == 0 {
            return Err(StoreError::MeetingNotFound(id));
        }
        drop(conn);
        self.meeting(id)
    }

    pub fn meeting(&self, id: i64) -> Result<Meeting, StoreError> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, name, goal, notes, started_at_ms, ended_at_ms,
                    stt_engine, stt_model, my_language, their_language
             FROM meetings WHERE id = ?1",
            params![id],
            meeting_from_row,
        )
        .optional()?
        .ok_or(StoreError::MeetingNotFound(id))
    }

    /// Frasi finali in ordine cronologico; il testo è letto dal file.
    pub fn segments(&self, meeting_id: i64) -> Result<Vec<Segment>, StoreError> {
        let conn = self.conn();
        let lines = transcript::read_lines(&transcript_path(&self.dir, &conn, meeting_id)?)?;
        let mut stmt = conn.prepare(
            "SELECT id, meeting_id, session_id, channel, speaker, text_line,
                    start_ms, end_ms, language, confidence
             FROM segments WHERE meeting_id = ?1 ORDER BY start_ms, id",
        )?;
        let rows = stmt.query_map(params![meeting_id], |row| {
            Ok(Segment {
                id: row.get(0)?,
                meeting_id: row.get(1)?,
                session_id: row.get::<_, i64>(2)? as u64,
                channel: row.get(3)?,
                speaker: row.get::<_, Option<i64>>(4)?.map(|s| s as u8),
                text: line_text(&lines, row.get(5)?),
                start_ms: row.get::<_, i64>(6)? as u64,
                end_ms: row.get::<_, i64>(7)? as u64,
                language: row.get(8)?,
                confidence: row.get::<_, Option<f64>>(9)?.map(|c| c as f32),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn translations(&self, meeting_id: i64) -> Result<Vec<Translation>, StoreError> {
        let conn = self.conn();
        let lines = transcript::read_lines(&transcript_path(&self.dir, &conn, meeting_id)?)?;
        let mut stmt = conn.prepare(
            "SELECT t.id, t.segment_id, s.session_id, t.text_line, t.language, t.latency_ms
             FROM translations t
             JOIN segments s ON s.id = t.segment_id
             WHERE s.meeting_id = ?1
             ORDER BY t.id",
        )?;
        let rows = stmt.query_map(params![meeting_id], |row| {
            Ok(Translation {
                id: row.get(0)?,
                segment_id: row.get(1)?,
                session_id: row.get::<_, i64>(2)? as u64,
                text: line_text(&lines, row.get(3)?),
                language: row.get(4)?,
                latency_ms: row.get::<_, i64>(5)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn speakers(&self, meeting_id: i64) -> Result<Vec<SpeakerName>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT speaker, name FROM speakers WHERE meeting_id = ?1 ORDER BY speaker")?;
        let rows = stmt.query_map(params![meeting_id], |row| {
            Ok(SpeakerName {
                speaker: row.get::<_, i64>(0)? as u8,
                name: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn assists(&self, meeting_id: i64) -> Result<Vec<Assist>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, kind, text, at_ms FROM assists
             WHERE meeting_id = ?1 ORDER BY at_ms, id",
        )?;
        let rows = stmt.query_map(params![meeting_id], |row| {
            Ok(Assist {
                id: row.get(0)?,
                kind: row.get(1)?,
                text: row.get(2)?,
                at_ms: row.get::<_, i64>(3)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Blocchi di compressione in ordine cronologico.
    pub fn compression_blocks(&self, meeting_id: i64) -> Result<Vec<CompressionBlock>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, meeting_id, block_index, start_ms, end_ms, summary, topics, moments
             FROM compression_blocks WHERE meeting_id = ?1 ORDER BY block_index",
        )?;
        let rows = stmt.query_map(params![meeting_id], |row| {
            Ok(CompressionBlock {
                id: row.get(0)?,
                meeting_id: row.get(1)?,
                index: u32::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                start_ms: row.get::<_, i64>(3)? as u64,
                end_ms: row.get::<_, i64>(4)? as u64,
                summary: row.get(5)?,
                topics: json_column(row.get(6)?),
                moments: json_column(row.get(7)?),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn summary(&self, meeting_id: i64) -> Result<Option<Summary>, StoreError> {
        let conn = self.conn();
        conn.query_row(
            "SELECT meeting_id, language, overview, key_points, action_items, created_at_ms
             FROM summaries WHERE meeting_id = ?1",
            params![meeting_id],
            |row| {
                Ok(Summary {
                    meeting_id: row.get(0)?,
                    language: row.get(1)?,
                    overview: row.get(2)?,
                    key_points: json_column(row.get(3)?),
                    action_items: json_column(row.get(4)?),
                    created_at_ms: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    /// Salva (o sostituisce) il riepilogo del meeting.
    pub fn save_summary(
        &self,
        meeting_id: i64,
        summary: &NewSummary,
    ) -> Result<Summary, StoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        upsert_summary_tx(&tx, meeting_id, now_ms(), summary)?;
        tx.commit()?;
        drop(conn);
        self.summary(meeting_id)?
            .ok_or(StoreError::MeetingNotFound(meeting_id))
    }

    /// Aggiorna lo stato spuntato delle voci della checklist.
    pub fn set_action_items(
        &self,
        meeting_id: i64,
        items: &[ActionItem],
    ) -> Result<Summary, StoreError> {
        let json = serde_json::to_string(items).unwrap_or_else(|_| "[]".into());
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE summaries SET action_items = ?1 WHERE meeting_id = ?2",
            params![json, meeting_id],
        )?;
        drop(conn);
        if changed == 0 {
            return Err(StoreError::MeetingNotFound(meeting_id));
        }
        self.summary(meeting_id)?
            .ok_or(StoreError::MeetingNotFound(meeting_id))
    }

    /// Meeting completo di frasi, traduzioni, parlanti, assist e riepilogo.
    pub fn details(&self, meeting_id: i64) -> Result<MeetingDetails, StoreError> {
        Ok(MeetingDetails {
            meeting: self.meeting(meeting_id)?,
            segments: self.segments(meeting_id)?,
            translations: self.translations(meeting_id)?,
            speakers: self.speakers(meeting_id)?,
            assists: self.assists(meeting_id)?,
            blocks: self.compression_blocks(meeting_id)?,
            summary: self.summary(meeting_id)?,
        })
    }
}

pub(crate) fn upsert_block_tx(
    tx: &rusqlite::Transaction<'_>,
    meeting_id: i64,
    now_ms: i64,
    block: &NewCompressionBlock,
) -> Result<(), StoreError> {
    let topics = serde_json::to_string(&block.topics).unwrap_or_else(|_| "[]".into());
    let moments = serde_json::to_string(&block.moments).unwrap_or_else(|_| "[]".into());
    tx.execute(
        "INSERT INTO compression_blocks
             (meeting_id, block_index, start_ms, end_ms, summary, topics, moments, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(meeting_id, block_index) DO UPDATE SET
             start_ms = excluded.start_ms,
             end_ms = excluded.end_ms,
             summary = excluded.summary,
             topics = excluded.topics,
             moments = excluded.moments,
             created_at_ms = excluded.created_at_ms",
        params![
            meeting_id,
            i64::from(block.index),
            block.start_ms as i64,
            block.end_ms as i64,
            block.summary,
            topics,
            moments,
            now_ms
        ],
    )?;
    Ok(())
}

pub(crate) fn upsert_summary_tx(
    tx: &rusqlite::Transaction<'_>,
    meeting_id: i64,
    now_ms: i64,
    summary: &NewSummary,
) -> Result<(), StoreError> {
    let key_points = serde_json::to_string(&summary.key_points).unwrap_or_else(|_| "[]".into());
    let action_items = serde_json::to_string(&summary.action_items).unwrap_or_else(|_| "[]".into());
    tx.execute(
        "INSERT INTO summaries
             (meeting_id, language, overview, key_points, action_items, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(meeting_id) DO UPDATE SET
             language = excluded.language,
             overview = excluded.overview,
             key_points = excluded.key_points,
             action_items = excluded.action_items,
             created_at_ms = excluded.created_at_ms",
        params![
            meeting_id,
            summary.language,
            summary.overview,
            key_points,
            action_items,
            now_ms
        ],
    )?;
    Ok(())
}

fn json_column<T: serde::de::DeserializeOwned + Default>(raw: String) -> T {
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!("dato JSON non valido nel database: {e}");
        T::default()
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn meeting_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Meeting> {
    Ok(Meeting {
        id: row.get(0)?,
        name: row.get(1)?,
        goal: row.get(2)?,
        notes: row.get(3)?,
        started_at_ms: row.get(4)?,
        ended_at_ms: row.get(5)?,
        stt_engine: row.get(6)?,
        stt_model: row.get(7)?,
        my_language: row.get(8)?,
        their_language: row.get(9)?,
    })
}

fn meeting_summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingSummary> {
    Ok(MeetingSummary {
        meeting: meeting_from_row(row)?,
        segment_count: u32::try_from(row.get::<_, i64>(10)?).unwrap_or(u32::MAX),
    })
}

/// Riferimento al file di trascrizione di un meeting.
fn transcript_path(dir: &Path, conn: &Connection, meeting_id: i64) -> Result<PathBuf, StoreError> {
    let file: Option<String> = conn
        .query_row(
            "SELECT transcript_file FROM meetings WHERE id = ?1",
            params![meeting_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(file) = file else {
        return Err(StoreError::MeetingNotFound(meeting_id));
    };
    Ok(if file.is_empty() {
        dir.join(transcript::relative_path(meeting_id))
    } else {
        dir.join(file)
    })
}

/// Testo di una riga (1-based); riferimento rotto = stringa vuota.
fn line_text(lines: &[String], line: Option<i64>) -> String {
    line.and_then(|line| usize::try_from(line).ok())
        .and_then(|line| line.checked_sub(1))
        .and_then(|index| lines.get(index))
        .cloned()
        .unwrap_or_default()
}

/// Parole della ricerca, normalizzate (minuscole, senza accenti).
fn search_terms(input: &str) -> Vec<String> {
    normalize(input)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Vero se il file contiene tutte le parole, come prefisso di un token.
fn lines_match(lines: &[String], terms: &[String]) -> bool {
    let mut found = vec![false; terms.len()];
    let mut missing = terms.len();
    for line in lines {
        for token in normalize(line)
            .split(|c: char| !c.is_alphanumeric())
            .filter(|token| !token.is_empty())
        {
            for (index, term) in terms.iter().enumerate() {
                if !found[index] && token.starts_with(term.as_str()) {
                    found[index] = true;
                    missing -= 1;
                }
            }
        }
        if missing == 0 {
            return true;
        }
    }
    missing == 0
}

fn normalize(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(fold)
        .collect()
}

/// Riduce le lettere accentate latine alla forma semplice, così "città" e
/// "citta" si trovano a vicenda (come faceva FTS5 con remove_diacritics).
fn fold(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ĭ' | 'į' => 'i',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ý' | 'ÿ' => 'y',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::SCHEMA_VERSION;

    fn user_version(path: &Path) -> i64 {
        let conn = Connection::open(path).unwrap();
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn migration_sets_version_and_creates_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sottovoce.db");
        let store = Store::open(&path).unwrap();
        assert!(store.meetings().unwrap().is_empty());
        drop(store);
        assert_eq!(user_version(&path), SCHEMA_VERSION);

        let conn = Connection::open(&path).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name IN
                     ('meetings', 'segments', 'translations', 'speakers', 'assists')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 5);
        // Il testo non è più una colonna delle tabelle.
        let text_columns: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('segments') WHERE name = 'text'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(text_columns, 0);
    }

    #[test]
    fn v3_database_moves_transcript_text_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sottovoce.db");
        {
            let conn = Connection::open(&path).unwrap();
            for sql in crate::schema::MIGRATIONS.iter().take(3) {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 3).unwrap();
            conn.execute(
                "INSERT INTO meetings (name, started_at_ms, stt_engine, stt_model)
                 VALUES ('vecchio', 1000, 'deepgram', 'nova-3')",
                [],
            )
            .unwrap();
            let meeting_id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO segments
                     (meeting_id, session_id, channel, text, start_ms, end_ms, created_at_ms)
                 VALUES (?1, 1, 'me', 'ciao a tutti', 0, 1000, 0)",
                params![meeting_id],
            )
            .unwrap();
            let segment_id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO translations
                     (segment_id, text, language, latency_ms, created_at_ms)
                 VALUES (?1, 'hello everyone', 'en', 10, 0)",
                params![segment_id],
            )
            .unwrap();
        }

        let store = Store::open(&path).unwrap();
        assert_eq!(user_version(&path), SCHEMA_VERSION);
        let details = store.details(1).unwrap();
        assert_eq!(details.segments[0].text, "ciao a tutti");
        assert_eq!(details.translations[0].text, "hello everyone");

        let file = std::fs::read_to_string(dir.path().join("transcripts/1.txt")).unwrap();
        assert_eq!(file, "ciao a tutti\nhello everyone\n");
        drop(store);

        // Il testo non è più nel database né nel WAL: la migrazione fa VACUUM.
        let db = std::fs::read(&path).unwrap();
        assert!(!db.windows(7).any(|w| w == b"ciao a "));
        let wal = std::fs::read(path.with_extension("db-wal")).unwrap_or_default();
        assert!(!wal.windows(7).any(|w| w == b"ciao a "));
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sottovoce.db");
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(conn);
        assert!(matches!(
            Store::open(&path),
            Err(StoreError::SchemaTooNew { .. })
        ));
    }

    #[test]
    fn recovers_a_meeting_left_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sottovoce.db");
        let conn = schema::open_connection(&path).unwrap();
        conn.execute(
            "INSERT INTO meetings (name, started_at_ms, stt_engine, stt_model)
             VALUES ('crash', 1000, 'local', 'parakeet')",
            [],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO segments
                 (meeting_id, session_id, channel, text_line, start_ms, end_ms, created_at_ms)
             VALUES (?1, 7, 'them', 1, 42_000, 45_000, 0)",
            params![id],
        )
        .unwrap();
        drop(conn);

        let store = Store::open(&path).unwrap();
        assert_eq!(store.recover_open_meetings().unwrap(), 1);
        let meeting = store.meeting(id).unwrap();
        assert_eq!(meeting.ended_at_ms, Some(46_000));
        // Un secondo avvio non cambia la fine già registrata.
        assert_eq!(store.recover_open_meetings().unwrap(), 0);
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.meeting(id).unwrap().ended_at_ms, Some(46_000));
    }
}
