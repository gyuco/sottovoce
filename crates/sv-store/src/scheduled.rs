//! Riunioni programmate: giorno, ora e contesto preparati in anticipo.
//!
//! All'ora della riunione l'app propone di avviarla con il contesto già
//! compilato (nome, obiettivo, note, partecipanti, cartella del codice e
//! lingua della call). Avviata la sessione la riga si elimina: da lì in poi il
//! meeting vive nello storico.
//!
//! Le riunioni importate da un calendario (`source` = `google`) hanno anche
//! `external_id`: la sincronizzazione ([`Store::sync_external_meetings`])
//! aggiorna titolo, orario e link, e partecipanti/note solo finché l'utente
//! non ha modificato la riunione (`edited`). Eliminarle le nasconde
//! (`hidden`), così la sincronizzazione successiva non le fa ricomparire.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;
use crate::store::{Store, now_ms};

pub const MAX_NAME_CHARS: usize = 200;
pub const MAX_GOAL_CHARS: usize = 500;
pub const MAX_NOTES_CHARS: usize = 2000;
pub const MAX_CODE_PATH_CHARS: usize = 1024;
pub const MAX_PARTICIPANTS: usize = 20;
pub const MIN_DURATION_MIN: u32 = 5;
pub const MAX_DURATION_MIN: u32 = 480;
/// Lingue della call accettate (come `Language` dell'app).
pub const LANGUAGES: &[&str] = &["multi", "it", "en", "es", "fr", "de"];

/// Partecipante atteso (ruolo e descrizione copiati dalla rubrica).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScheduledParticipant {
    pub name: String,
    pub role: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledMeeting {
    pub id: i64,
    pub name: String,
    pub goal: String,
    pub notes: String,
    pub code_path: String,
    pub participants: Vec<ScheduledParticipant>,
    pub language: String,
    pub starts_at_ms: i64,
    pub duration_min: u32,
    /// Calendario di provenienza (`google`); vuoto = creata a mano.
    pub source: String,
    /// Link della call (Meet, Teams, Zoom), se noto.
    pub join_url: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Evento di un calendario esterno da importare.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalMeeting {
    pub external_id: String,
    pub name: String,
    pub notes: String,
    pub participants: Vec<ScheduledParticipant>,
    pub starts_at_ms: i64,
    pub ends_at_ms: i64,
    pub join_url: String,
}

/// Esito di una sincronizzazione.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

/// Riunione da salvare: senza `id` si crea, con `id` si aggiorna.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScheduledMeetingInput {
    pub id: Option<i64>,
    pub name: String,
    pub goal: String,
    pub notes: String,
    pub code_path: String,
    pub participants: Vec<ScheduledParticipant>,
    pub language: String,
    pub starts_at_ms: i64,
    pub duration_min: u32,
}

const COLUMNS: &str = "id, name, goal, notes, code_path, participants, language, \
                       starts_at_ms, duration_min, source, join_url, created_at_ms, updated_at_ms";

impl Store {
    /// Riunioni che finiscono dopo `since_ms`, dalla più vicina.
    pub fn scheduled_meetings(&self, since_ms: i64) -> Result<Vec<ScheduledMeeting>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM scheduled_meetings
             WHERE starts_at_ms + duration_min * 60000 >= ?1 AND hidden = 0
             ORDER BY starts_at_ms, id"
        ))?;
        let rows = stmt.query_map(params![since_ms], scheduled_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn scheduled_meeting(&self, id: i64) -> Result<ScheduledMeeting, StoreError> {
        let conn = self.conn();
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM scheduled_meetings WHERE id = ?1"),
            params![id],
            scheduled_from_row,
        )
        .optional()?
        .ok_or(StoreError::ScheduledNotFound(id))
    }

    /// Crea o aggiorna una riunione programmata (vedi [`ScheduledMeetingInput`]).
    pub fn save_scheduled_meeting(
        &self,
        input: &ScheduledMeetingInput,
    ) -> Result<ScheduledMeeting, StoreError> {
        let name = clip(input.name.trim(), MAX_NAME_CHARS);
        if name.is_empty() {
            return Err(StoreError::EmptyName);
        }
        if !LANGUAGES.contains(&input.language.as_str()) {
            return Err(StoreError::InvalidScheduled(format!(
                "lingua sconosciuta: {}",
                input.language
            )));
        }
        if input.starts_at_ms <= 0 {
            return Err(StoreError::InvalidScheduled("data mancante".into()));
        }
        let duration = input.duration_min.clamp(MIN_DURATION_MIN, MAX_DURATION_MIN);
        let goal = clip(input.goal.trim(), MAX_GOAL_CHARS);
        let notes = clip(input.notes.trim(), MAX_NOTES_CHARS);
        let code_path = clip(input.code_path.trim(), MAX_CODE_PATH_CHARS);
        let participants: Vec<_> = input
            .participants
            .iter()
            .filter(|p| !p.name.trim().is_empty())
            .take(MAX_PARTICIPANTS)
            .cloned()
            .collect();
        let participants = serde_json::to_string(&participants)
            .map_err(|e| StoreError::InvalidScheduled(e.to_string()))?;
        let now = now_ms();

        let conn = self.conn();
        let id = match input.id {
            Some(id) => {
                let changed = conn.execute(
                    "UPDATE scheduled_meetings
                     SET name = ?1, goal = ?2, notes = ?3, code_path = ?4, participants = ?5,
                         language = ?6, starts_at_ms = ?7, duration_min = ?8, updated_at_ms = ?9,
                         edited = 1
                     WHERE id = ?10",
                    params![
                        name,
                        goal,
                        notes,
                        code_path,
                        participants,
                        input.language,
                        input.starts_at_ms,
                        duration,
                        now,
                        id
                    ],
                )?;
                if changed == 0 {
                    return Err(StoreError::ScheduledNotFound(id));
                }
                id
            }
            None => {
                conn.execute(
                    "INSERT INTO scheduled_meetings
                         (name, goal, notes, code_path, participants, language,
                          starts_at_ms, duration_min, created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                    params![
                        name,
                        goal,
                        notes,
                        code_path,
                        participants,
                        input.language,
                        input.starts_at_ms,
                        duration,
                        now
                    ],
                )?;
                conn.last_insert_rowid()
            }
        };
        drop(conn);
        self.scheduled_meeting(id)
    }

    /// Elimina una riunione; se viene da un calendario la nasconde soltanto,
    /// perché la sincronizzazione non la reimporti.
    pub fn delete_scheduled_meeting(&self, id: i64) -> Result<(), StoreError> {
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE scheduled_meetings SET hidden = 1 WHERE id = ?1 AND external_id IS NOT NULL",
            params![id],
        )? + conn.execute(
            "DELETE FROM scheduled_meetings WHERE id = ?1 AND external_id IS NULL",
            params![id],
        )?;
        if changed == 0 {
            return Err(StoreError::ScheduledNotFound(id));
        }
        Ok(())
    }

    /// Allinea le riunioni di `source` che iniziano in `[from_ms, to_ms)` agli
    /// eventi del calendario: nuove inserite (lingua `language`), esistenti
    /// aggiornate, quelle sparite (annullate, spostate fuori, rifiutate)
    /// eliminate. Le modifiche dell'utente a obiettivo, lingua, cartella del
    /// codice, e — dopo una modifica — a note e partecipanti restano.
    pub fn sync_external_meetings(
        &self,
        source: &str,
        from_ms: i64,
        to_ms: i64,
        events: &[ExternalMeeting],
        language: &str,
    ) -> Result<SyncReport, StoreError> {
        if source.is_empty() {
            return Err(StoreError::InvalidScheduled("sorgente mancante".into()));
        }
        let language = if LANGUAGES.contains(&language) {
            language
        } else {
            "multi"
        };
        let now = now_ms();
        let mut report = SyncReport::default();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut seen = std::collections::HashSet::new();
        for event in events {
            if event.external_id.is_empty() || !seen.insert(event.external_id.as_str()) {
                continue;
            }
            let name = clip(event.name.trim(), MAX_NAME_CHARS);
            let notes = clip(event.notes.trim(), MAX_NOTES_CHARS);
            let participants: Vec<_> = event
                .participants
                .iter()
                .filter(|p| !p.name.trim().is_empty())
                .take(MAX_PARTICIPANTS)
                .cloned()
                .collect();
            let participants = serde_json::to_string(&participants)
                .map_err(|e| StoreError::InvalidScheduled(e.to_string()))?;
            let minutes = (event.ends_at_ms - event.starts_at_ms).max(0) / 60_000;
            let duration = u32::try_from(minutes)
                .unwrap_or(MAX_DURATION_MIN)
                .clamp(MIN_DURATION_MIN, MAX_DURATION_MIN);
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM scheduled_meetings
                     WHERE source = ?1 AND external_id = ?2",
                    params![source, event.external_id],
                    |row| row.get(0),
                )
                .optional()?;
            match existing {
                Some(id) => {
                    let changed = tx.execute(
                        "UPDATE scheduled_meetings
                         SET name = ?1, starts_at_ms = ?2, duration_min = ?3, join_url = ?4,
                             notes = CASE WHEN edited = 0 THEN ?5 ELSE notes END,
                             participants = CASE WHEN edited = 0 THEN ?6 ELSE participants END,
                             updated_at_ms = ?7
                         WHERE id = ?8 AND (name <> ?1 OR starts_at_ms <> ?2 OR duration_min <> ?3
                             OR join_url <> ?4
                             OR (edited = 0 AND (notes <> ?5 OR participants <> ?6)))",
                        params![
                            name,
                            event.starts_at_ms,
                            duration,
                            event.join_url,
                            notes,
                            participants,
                            now,
                            id
                        ],
                    )?;
                    report.updated += changed;
                }
                None => {
                    tx.execute(
                        "INSERT INTO scheduled_meetings
                             (name, notes, participants, language, starts_at_ms, duration_min,
                              source, external_id, join_url, created_at_ms, updated_at_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                        params![
                            name,
                            notes,
                            participants,
                            language,
                            event.starts_at_ms,
                            duration,
                            source,
                            event.external_id,
                            event.join_url,
                            now
                        ],
                    )?;
                    report.added += 1;
                }
            }
        }
        let stale: Vec<(i64, String)> = {
            let mut stmt = tx.prepare(
                "SELECT id, external_id FROM scheduled_meetings
                 WHERE source = ?1 AND external_id IS NOT NULL
                   AND starts_at_ms >= ?2 AND starts_at_ms < ?3",
            )?;
            stmt.query_map(params![source, from_ms, to_ms], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<_, _>>()?
        };
        for (id, external_id) in stale {
            if !seen.contains(external_id.as_str()) {
                report.removed +=
                    tx.execute("DELETE FROM scheduled_meetings WHERE id = ?1", params![id])?;
            }
        }
        tx.commit()?;
        Ok(report)
    }

    /// Calendario scollegato: via le sue riunioni, tranne quelle modificate
    /// dall'utente, che restano come riunioni create a mano.
    pub fn forget_external_meetings(&self, source: &str) -> Result<usize, StoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let removed = tx.execute(
            "DELETE FROM scheduled_meetings WHERE source = ?1 AND (edited = 0 OR hidden = 1)",
            params![source],
        )?;
        tx.execute(
            "UPDATE scheduled_meetings SET source = '', external_id = NULL WHERE source = ?1",
            params![source],
        )?;
        tx.commit()?;
        Ok(removed)
    }

    /// Elimina le riunioni finite prima di `before_ms` e mai avviate.
    pub fn prune_scheduled_meetings(&self, before_ms: i64) -> Result<usize, StoreError> {
        let conn = self.conn();
        Ok(conn.execute(
            "DELETE FROM scheduled_meetings WHERE starts_at_ms + duration_min * 60000 < ?1",
            params![before_ms],
        )?)
    }
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect::<String>().trim().to_owned()
}

fn scheduled_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduledMeeting> {
    let participants: String = row.get(5)?;
    Ok(ScheduledMeeting {
        id: row.get(0)?,
        name: row.get(1)?,
        goal: row.get(2)?,
        notes: row.get(3)?,
        code_path: row.get(4)?,
        // Un JSON illeggibile non deve nascondere la riunione.
        participants: serde_json::from_str(&participants).unwrap_or_default(),
        language: row.get(6)?,
        starts_at_ms: row.get(7)?,
        duration_min: row.get(8)?,
        source: row.get(9)?,
        join_url: row.get(10)?,
        created_at_ms: row.get(11)?,
        updated_at_ms: row.get(12)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("sottovoce.db")).unwrap();
        (dir, store)
    }

    fn input(name: &str, starts_at_ms: i64) -> ScheduledMeetingInput {
        ScheduledMeetingInput {
            name: name.into(),
            language: "it".into(),
            starts_at_ms,
            duration_min: 30,
            ..ScheduledMeetingInput::default()
        }
    }

    #[test]
    fn creates_updates_lists_and_deletes() {
        let (_dir, store) = store();
        let later = store
            .save_scheduled_meeting(&ScheduledMeetingInput {
                goal: " chiudere il budget ".into(),
                participants: vec![
                    ScheduledParticipant {
                        name: "Anna".into(),
                        role: "CTO".into(),
                        ..Default::default()
                    },
                    ScheduledParticipant::default(),
                ],
                duration_min: 1,
                ..input(" Sprint review ", 1_000 * MIN)
            })
            .unwrap();
        assert_eq!(later.name, "Sprint review");
        assert_eq!(later.goal, "chiudere il budget");
        assert_eq!(later.duration_min, MIN_DURATION_MIN);
        assert_eq!(later.participants.len(), 1);
        assert_eq!(later.participants[0].role, "CTO");

        let sooner = store
            .save_scheduled_meeting(&input("Standup", 500 * MIN))
            .unwrap();
        let names: Vec<_> = store
            .scheduled_meetings(0)
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(names, ["Standup", "Sprint review"]);

        let moved = store
            .save_scheduled_meeting(&ScheduledMeetingInput {
                id: Some(sooner.id),
                ..input("Standup", 2_000 * MIN)
            })
            .unwrap();
        assert_eq!(moved.id, sooner.id);
        assert_eq!(store.scheduled_meetings(0).unwrap()[1].id, sooner.id);

        store.delete_scheduled_meeting(later.id).unwrap();
        assert!(matches!(
            store.delete_scheduled_meeting(later.id),
            Err(StoreError::ScheduledNotFound(_))
        ));
        assert!(matches!(
            store.save_scheduled_meeting(&ScheduledMeetingInput {
                id: Some(later.id),
                ..input("x", MIN)
            }),
            Err(StoreError::ScheduledNotFound(_))
        ));
    }

    #[test]
    fn filters_by_end_and_prunes() {
        let (_dir, store) = store();
        // 100–130 min e 200–230 min.
        store
            .save_scheduled_meeting(&input("a", 100 * MIN))
            .unwrap();
        store
            .save_scheduled_meeting(&input("b", 200 * MIN))
            .unwrap();
        // In corso a 120 min: resta visibile fino alla fine prevista.
        assert_eq!(store.scheduled_meetings(120 * MIN).unwrap().len(), 2);
        assert_eq!(store.scheduled_meetings(131 * MIN).unwrap().len(), 1);
        assert_eq!(store.prune_scheduled_meetings(131 * MIN).unwrap(), 1);
        assert_eq!(store.scheduled_meetings(0).unwrap().len(), 1);
    }

    fn event(id: &str, name: &str, start: i64) -> ExternalMeeting {
        ExternalMeeting {
            external_id: id.into(),
            name: name.into(),
            notes: "agenda".into(),
            participants: vec![ScheduledParticipant {
                name: "Anna".into(),
                ..Default::default()
            }],
            starts_at_ms: start,
            ends_at_ms: start + 45 * MIN,
            join_url: "https://meet.google.com/abc-defg-hij".into(),
        }
    }

    #[test]
    fn syncs_external_meetings() {
        let (_dir, store) = store();
        let manual = store
            .save_scheduled_meeting(&input("A mano", 150 * MIN))
            .unwrap();
        let first = [
            event("e1", "Review", 100 * MIN),
            event("e2", "Standup", 200 * MIN),
        ];
        let report = store
            .sync_external_meetings("google", 0, 1_000 * MIN, &first, "en")
            .unwrap();
        assert_eq!((report.added, report.updated, report.removed), (2, 0, 0));
        let all = store.scheduled_meetings(0).unwrap();
        assert_eq!(all.len(), 3);
        let review = &all[0];
        assert_eq!(
            (review.source.as_str(), review.language.as_str()),
            ("google", "en")
        );
        assert_eq!(review.duration_min, 45);
        assert_eq!(review.join_url, "https://meet.google.com/abc-defg-hij");

        // Stessi dati: nessun aggiornamento.
        let again = store
            .sync_external_meetings("google", 0, 1_000 * MIN, &first, "en")
            .unwrap();
        assert_eq!(again, SyncReport::default());

        // L'utente modifica la review (obiettivo e note) e nasconde lo standup.
        store
            .save_scheduled_meeting(&ScheduledMeetingInput {
                id: Some(review.id),
                goal: "decidere".into(),
                notes: "mie note".into(),
                language: "it".into(),
                ..input("Review", review.starts_at_ms)
            })
            .unwrap();
        let standup = all.iter().find(|m| m.name == "Standup").unwrap().id;
        store.delete_scheduled_meeting(standup).unwrap();

        // Il calendario sposta la review e cambia le note; lo standup resta.
        let mut moved = event("e1", "Review Q4", 120 * MIN);
        moved.notes = "nuova agenda".into();
        let report = store
            .sync_external_meetings(
                "google",
                0,
                1_000 * MIN,
                &[moved, event("e2", "Standup", 200 * MIN)],
                "en",
            )
            .unwrap();
        assert_eq!((report.added, report.updated, report.removed), (0, 1, 0));
        let all = store.scheduled_meetings(0).unwrap();
        assert_eq!(all.len(), 2, "lo standup nascosto non ricompare");
        let review = all.iter().find(|m| m.source == "google").unwrap();
        assert_eq!(review.name, "Review Q4");
        assert_eq!(review.starts_at_ms, 120 * MIN);
        assert_eq!(
            (
                review.goal.as_str(),
                review.notes.as_str(),
                review.language.as_str()
            ),
            ("decidere", "mie note", "it")
        );

        // La review sparisce dal calendario: eliminata. Fuori finestra nulla cambia.
        let report = store
            .sync_external_meetings("google", 0, 1_000 * MIN, &[], "en")
            .unwrap();
        assert_eq!(report.removed, 2);
        assert_eq!(
            store.scheduled_meetings(0).unwrap(),
            [store.scheduled_meeting(manual.id).unwrap()]
        );
    }

    #[test]
    fn forgetting_a_source_keeps_edited_meetings() {
        let (_dir, store) = store();
        store
            .sync_external_meetings(
                "google",
                0,
                1_000 * MIN,
                &[event("e1", "A", 10 * MIN), event("e2", "B", 20 * MIN)],
                "it",
            )
            .unwrap();
        let a = store.scheduled_meetings(0).unwrap()[0].clone();
        store
            .save_scheduled_meeting(&ScheduledMeetingInput {
                id: Some(a.id),
                goal: "x".into(),
                ..input("A", a.starts_at_ms)
            })
            .unwrap();
        assert_eq!(store.forget_external_meetings("google").unwrap(), 1);
        let left = store.scheduled_meetings(0).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!((left[0].name.as_str(), left[0].source.as_str()), ("A", ""));
        // Ora è una riunione a mano: eliminarla la cancella davvero.
        store.delete_scheduled_meeting(left[0].id).unwrap();
        assert!(store.scheduled_meetings(0).unwrap().is_empty());
    }

    #[test]
    fn rejects_invalid_input() {
        let (_dir, store) = store();
        assert!(matches!(
            store.save_scheduled_meeting(&input("  ", MIN)),
            Err(StoreError::EmptyName)
        ));
        assert!(matches!(
            store.save_scheduled_meeting(&ScheduledMeetingInput {
                language: "xx".into(),
                ..input("a", MIN)
            }),
            Err(StoreError::InvalidScheduled(_))
        ));
        assert!(matches!(
            store.save_scheduled_meeting(&input("a", 0)),
            Err(StoreError::InvalidScheduled(_))
        ));
        assert!(store.scheduled_meetings(0).unwrap().is_empty());
    }
}
