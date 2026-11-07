//! Lettura del database dei meeting.
//!
//! [`Store::open`] applica le migrazioni e chiude i meeting rimasti aperti
//! (crash o chiusura forzata dell'app). Le query sono sincrone e pensate per
//! l'avvio, lo storico e i futuri comandi Tauri.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::StoreError;
use crate::schema;
use crate::types::{
    Assist, Meeting, MeetingDetails, MeetingPage, MeetingSummary, Segment, SpeakerName, Translation,
};

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Apre (creando se serve) `sottovoce.db` e applica le migrazioni.
    ///
    /// La chiusura dei meeting rimasti appesi non è automatica: l'app la fa
    /// una volta all'avvio con [`Store::recover_open_meetings`], così una
    /// lettura durante una call non tocca il meeting in corso.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let conn = schema::open_connection(path.as_ref())?;
        Ok(Self {
            conn: Mutex::new(conn),
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

    /// Pagina dello storico. Con `search` filtra i meeting che contengono
    /// almeno una frase che soddisfa la ricerca full-text (FTS5).
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
        let Some(fts) = fts_query(query) else {
            return Ok(MeetingPage {
                meetings: Vec::new(),
                total: 0,
            });
        };
        let conn = self.conn();
        let total: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT s.meeting_id)
             FROM segments_fts JOIN segments s ON s.id = segments_fts.rowid
             WHERE segments_fts MATCH ?1",
            params![fts],
            |row| row.get(0),
        )?;
        let mut stmt = conn.prepare(
            "SELECT m.id, m.name, m.goal, m.notes, m.started_at_ms, m.ended_at_ms,
                    m.stt_engine, m.stt_model, m.my_language, m.their_language,
                    (SELECT COUNT(*) FROM segments x WHERE x.meeting_id = m.id)
             FROM meetings m
             WHERE m.id IN (
                 SELECT s.meeting_id
                 FROM segments_fts JOIN segments s ON s.id = segments_fts.rowid
                 WHERE segments_fts MATCH ?1
             )
             ORDER BY m.started_at_ms DESC, m.id DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(
            params![fts, i64::from(limit), i64::from(offset)],
            meeting_summary_from_row,
        )?;
        Ok(MeetingPage {
            meetings: rows.collect::<Result<Vec<_>, _>>()?,
            total: u32::try_from(total).unwrap_or(u32::MAX),
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

    /// Elimina meeting, frasi (e indice di ricerca), traduzioni, parlanti e
    /// assist in cascata.
    pub fn delete_meeting(&self, id: i64) -> Result<(), StoreError> {
        let conn = self.conn();
        let deleted = conn.execute("DELETE FROM meetings WHERE id = ?1", params![id])?;
        if deleted == 0 {
            return Err(StoreError::MeetingNotFound(id));
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

    /// Frasi finali in ordine cronologico.
    pub fn segments(&self, meeting_id: i64) -> Result<Vec<Segment>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, meeting_id, session_id, channel, speaker, text,
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
                text: row.get(5)?,
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
        let mut stmt = conn.prepare(
            "SELECT t.id, t.segment_id, s.session_id, t.text, t.language, t.latency_ms
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
                text: row.get(3)?,
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

    /// Meeting completo di frasi, traduzioni, parlanti e assist.
    pub fn details(&self, meeting_id: i64) -> Result<MeetingDetails, StoreError> {
        Ok(MeetingDetails {
            meeting: self.meeting(meeting_id)?,
            segments: self.segments(meeting_id)?,
            translations: self.translations(meeting_id)?,
            speakers: self.speakers(meeting_id)?,
            assists: self.assists(meeting_id)?,
        })
    }
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

/// Query FTS5 tollerante all'input dell'utente: ogni parola diventa un
/// prefisso tra virgolette (`"parola"*`), così punteggiatura e apostrofi non
/// fanno fallire la sintassi di MATCH.
fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"*", t.to_lowercase()))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" AND "))
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
                 (meeting_id, session_id, channel, text, start_ms, end_ms, created_at_ms)
             VALUES (?1, 7, 'them', 'ultima frase', 42_000, 45_000, 0)",
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
