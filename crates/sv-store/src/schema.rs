//! Schema e migrazioni del database.
//!
//! Ogni migrazione è applicata in una transazione e la versione corrente è
//! registrata in `PRAGMA user_version`: aprire un DB più recente dell'app è
//! un errore esplicito, mai una corruzione silenziosa.

use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::error::StoreError;

/// Versione dello schema corrente.
pub const SCHEMA_VERSION: i64 = 3;

const MIGRATIONS: &[&str] = &[
    // 1 — meeting, frasi finali, traduzioni, nomi dei parlanti, assist.
    r#"
CREATE TABLE meetings (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    name          TEXT    NOT NULL DEFAULT '',
    goal          TEXT    NOT NULL DEFAULT '',
    notes         TEXT    NOT NULL DEFAULT '',
    started_at_ms INTEGER NOT NULL,
    ended_at_ms   INTEGER,
    stt_engine    TEXT    NOT NULL DEFAULT '',
    stt_model     TEXT    NOT NULL DEFAULT '',
    my_language   TEXT    NOT NULL DEFAULT 'multi',
    their_language TEXT   NOT NULL DEFAULT 'multi'
);

CREATE TABLE segments (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    meeting_id   INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    session_id   INTEGER NOT NULL,
    channel      TEXT    NOT NULL,
    speaker      INTEGER,
    text         TEXT    NOT NULL,
    start_ms     INTEGER NOT NULL,
    end_ms       INTEGER NOT NULL,
    language     TEXT,
    confidence   REAL,
    created_at_ms INTEGER NOT NULL,
    UNIQUE (meeting_id, session_id)
);
CREATE INDEX segments_by_meeting ON segments(meeting_id, start_ms);

CREATE TABLE translations (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    segment_id   INTEGER NOT NULL REFERENCES segments(id) ON DELETE CASCADE,
    text         TEXT    NOT NULL,
    language     TEXT    NOT NULL,
    latency_ms   INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    UNIQUE (segment_id, language)
);
CREATE INDEX translations_by_segment ON translations(segment_id);

CREATE TABLE speakers (
    meeting_id INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    speaker    INTEGER NOT NULL,
    name       TEXT    NOT NULL,
    PRIMARY KEY (meeting_id, speaker)
);

CREATE TABLE assists (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    meeting_id   INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    kind         TEXT    NOT NULL,
    text         TEXT    NOT NULL,
    at_ms        INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX assists_by_meeting ON assists(meeting_id, at_ms);
"#,
    // 2 — ricerca testuale nelle frasi (FTS5, contenuto esterno a `segments`).
    r#"
CREATE VIRTUAL TABLE segments_fts USING fts5(
    text,
    content = 'segments',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER segments_fts_ai AFTER INSERT ON segments BEGIN
    INSERT INTO segments_fts(rowid, text) VALUES (new.id, new.text);
END;

CREATE TRIGGER segments_fts_ad AFTER DELETE ON segments BEGIN
    INSERT INTO segments_fts(segments_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;

CREATE TRIGGER segments_fts_au AFTER UPDATE ON segments BEGIN
    INSERT INTO segments_fts(segments_fts, rowid, text) VALUES ('delete', old.id, old.text);
    INSERT INTO segments_fts(rowid, text) VALUES (new.id, new.text);
END;

INSERT INTO segments_fts(segments_fts) VALUES ('rebuild');
"#,
    // 3 — blocchi di compressione (5 min) e riepilogo finale.
    r#"
CREATE TABLE compression_blocks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    meeting_id   INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    block_index  INTEGER NOT NULL,
    start_ms     INTEGER NOT NULL,
    end_ms       INTEGER NOT NULL,
    summary      TEXT    NOT NULL,
    topics       TEXT    NOT NULL,
    moments      TEXT    NOT NULL,
    created_at_ms INTEGER NOT NULL,
    UNIQUE (meeting_id, block_index)
);

CREATE TABLE summaries (
    meeting_id   INTEGER PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
    language     TEXT NOT NULL,
    overview     TEXT NOT NULL,
    key_points   TEXT NOT NULL,
    action_items TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
"#,
];

/// Apre una connessione configurata (WAL, foreign key, busy timeout) e applica
/// le migrazioni mancanti.
pub(crate) fn open_connection(path: &Path) -> Result<Connection, StoreError> {
    if path != Path::new(":memory:")
        && let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    let mut conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )?;
    migrate(&mut conn)?;
    Ok(conn)
}

pub(crate) fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let found: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if found > SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
        let version = index as i64 + 1;
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        tracing::info!(version, "migrazione del database applicata");
    }
    Ok(())
}

/// Chiude i meeting rimasti aperti (app terminata durante una call):
/// `fine = avvio + ultimo evento noto` (0 se non c'è nulla).
pub(crate) fn close_open_meetings(conn: &mut Connection) -> Result<usize, StoreError> {
    let tx = conn.transaction()?;
    let open: Vec<(i64, i64, i64)> = {
        let mut stmt = tx.prepare(
            "SELECT m.id,
                    m.started_at_ms,
                    MAX(
                        COALESCE((SELECT MAX(end_ms) FROM segments s WHERE s.meeting_id = m.id), 0),
                        COALESCE((SELECT MAX(at_ms) FROM assists a WHERE a.meeting_id = m.id), 0)
                    )
             FROM meetings m
             WHERE m.ended_at_ms IS NULL",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (id, started_at_ms, last_ms) in &open {
        tx.execute(
            "UPDATE meetings SET ended_at_ms = ? WHERE id = ? AND ended_at_ms IS NULL",
            rusqlite::params![started_at_ms.saturating_add(*last_ms), id],
        )?;
    }
    tx.commit()?;
    for (id, _, last_ms) in &open {
        tracing::info!(
            meeting = id,
            last_ms,
            "meeting rimasto aperto: chiuso al riavvio"
        );
    }
    Ok(open.len())
}
