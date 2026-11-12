//! Riunioni programmate: giorno, ora e contesto preparati in anticipo.
//!
//! All'ora della riunione l'app propone di avviarla con il contesto già
//! compilato (nome, obiettivo, note, partecipanti, cartella del codice e
//! lingua della call). Avviata la sessione la riga si elimina: da lì in poi il
//! meeting vive nello storico.

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
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
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
                       starts_at_ms, duration_min, created_at_ms, updated_at_ms";

impl Store {
    /// Riunioni che finiscono dopo `since_ms`, dalla più vicina.
    pub fn scheduled_meetings(&self, since_ms: i64) -> Result<Vec<ScheduledMeeting>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM scheduled_meetings
             WHERE starts_at_ms + duration_min * 60000 >= ?1
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
                         language = ?6, starts_at_ms = ?7, duration_min = ?8, updated_at_ms = ?9
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

    pub fn delete_scheduled_meeting(&self, id: i64) -> Result<(), StoreError> {
        let conn = self.conn();
        if conn.execute("DELETE FROM scheduled_meetings WHERE id = ?1", params![id])? == 0 {
            return Err(StoreError::ScheduledNotFound(id));
        }
        Ok(())
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
        created_at_ms: row.get(9)?,
        updated_at_ms: row.get(10)?,
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
