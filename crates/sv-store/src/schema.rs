//! Schema e migrazioni del database.
//!
//! Ogni migrazione è applicata in una transazione e la versione corrente è
//! registrata in `PRAGMA user_version`: aprire un DB più recente dell'app è
//! un errore esplicito, mai una corruzione silenziosa.
//!
//! Il testo delle frasi e delle traduzioni non vive nel database: dalla
//! versione 4 è su file (`transcripts/<meeting_id>.txt`) e le tabelle
//! conservano solo il numero di riga.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, Transaction, params};

use crate::error::StoreError;
use crate::transcript;

/// Versione dello schema corrente.
pub const SCHEMA_VERSION: i64 = 7;

pub(crate) const MIGRATIONS: &[&str] = &[
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
    // 4 — il testo delle frasi/traduzioni passa su file (transcript.rs): il
    // DB conserva solo il riferimento (file del meeting + numero di riga).
    // Lo spostamento dei dati e la rimozione delle colonne `text` avvengono
    // in `move_text_to_files`, dopo questa DDL.
    r#"
DROP TRIGGER IF EXISTS segments_fts_ai;
DROP TRIGGER IF EXISTS segments_fts_ad;
DROP TRIGGER IF EXISTS segments_fts_au;
DROP TABLE IF EXISTS segments_fts;

ALTER TABLE meetings ADD COLUMN transcript_file TEXT NOT NULL DEFAULT '';
ALTER TABLE segments ADD COLUMN text_line INTEGER;
ALTER TABLE translations ADD COLUMN text_line INTEGER;
"#,
    // 5 — rubrica: schede delle persone incontrate nei meeting. Il legame
    // con i parlanti è il nome (`name_key` = nome normalizzato, unico).
    r#"
CREATE TABLE contacts (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    name          TEXT    NOT NULL,
    name_key      TEXT    NOT NULL UNIQUE,
    role          TEXT    NOT NULL DEFAULT '',
    description   TEXT    NOT NULL DEFAULT '',
    email         TEXT    NOT NULL DEFAULT '',
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
"#,
    // 6 — partecipanti attesi indicati all'avvio (array JSON di nomi).
    r#"
ALTER TABLE meetings ADD COLUMN participants TEXT NOT NULL DEFAULT '[]';
"#,
    // 7 — riunioni programmate: giorno, ora e contesto pronti per l'avvio.
    r#"
CREATE TABLE scheduled_meetings (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    name          TEXT    NOT NULL,
    goal          TEXT    NOT NULL DEFAULT '',
    notes         TEXT    NOT NULL DEFAULT '',
    code_path     TEXT    NOT NULL DEFAULT '',
    participants  TEXT    NOT NULL DEFAULT '[]',
    language      TEXT    NOT NULL DEFAULT 'multi',
    starts_at_ms  INTEGER NOT NULL,
    duration_min  INTEGER NOT NULL DEFAULT 30,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX scheduled_meetings_by_start ON scheduled_meetings(starts_at_ms);
"#,
];

/// Cartella che contiene il database e i file di trascrizione.
pub(crate) fn base_dir(db_path: &Path) -> PathBuf {
    if db_path == Path::new(":memory:") {
        return PathBuf::from(".");
    }
    db_path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

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
    migrate(&mut conn, &base_dir(path))?;
    Ok(conn)
}

pub(crate) fn migrate(conn: &mut Connection, base_dir: &Path) -> Result<(), StoreError> {
    let found: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if found > SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    let mut purge = false;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
        let version = index as i64 + 1;
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        if version == 4 {
            move_text_to_files(&tx, base_dir)?;
            purge = true;
        }
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        tracing::info!(version, "migrazione del database applicata");
    }
    if purge && let Err(e) = conn.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);") {
        // Il VACUUM è solo per non lasciare il testo migrato nelle vecchie
        // pagine: se fallisce (DB occupato) l'app deve partire lo stesso.
        tracing::warn!("pulizia del database dopo la migrazione non riuscita: {e}");
    }
    Ok(())
}

/// Migrazione 4: scrive su file il testo già presente nel DB e sostituisce le
/// colonne `text` con `text_line`. È idempotente: se la transazione non viene
/// committata, al prossimo avvio il file viene ricreato da capo.
fn move_text_to_files(tx: &Transaction<'_>, base_dir: &Path) -> Result<(), StoreError> {
    let meetings: Vec<i64> = {
        let mut stmt = tx.prepare("SELECT id FROM meetings ORDER BY id")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    for meeting_id in meetings {
        let relative = transcript::relative_path(meeting_id);
        let mut file = transcript::TranscriptFile::create(&base_dir.join(&relative))?;
        let segments: Vec<(i64, String)> = {
            let mut stmt = tx.prepare(
                "SELECT id, text FROM segments WHERE meeting_id = ?1 ORDER BY start_ms, id",
            )?;
            let rows = stmt.query_map(params![meeting_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        for (segment_id, text) in segments {
            let line = file.save(None, &text)?;
            tx.execute(
                "UPDATE segments SET text_line = ?1 WHERE id = ?2",
                params![to_i64(line), segment_id],
            )?;
            let translations: Vec<(i64, String)> = {
                let mut stmt = tx.prepare(
                    "SELECT id, text FROM translations WHERE segment_id = ?1 ORDER BY id",
                )?;
                let rows =
                    stmt.query_map(params![segment_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
                rows.collect::<Result<_, _>>()?
            };
            for (translation_id, text) in translations {
                let line = file.save(None, &text)?;
                tx.execute(
                    "UPDATE translations SET text_line = ?1 WHERE id = ?2",
                    params![to_i64(line), translation_id],
                )?;
            }
        }
        tx.execute(
            "UPDATE meetings SET transcript_file = ?1 WHERE id = ?2",
            params![relative.to_string_lossy(), meeting_id],
        )?;
    }
    tx.execute_batch(
        "ALTER TABLE segments DROP COLUMN text;
         ALTER TABLE translations DROP COLUMN text;",
    )?;
    Ok(())
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
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
