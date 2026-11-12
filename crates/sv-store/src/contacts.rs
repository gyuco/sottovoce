//! Rubrica: una scheda (nome, ruolo, descrizione, email) per le persone
//! incontrate nei meeting.
//!
//! Il legame con i parlanti di un meeting è il nome: `name_key` è il nome
//! normalizzato (minuscole, senza accenti, spazi compattati) ed è unico, così
//! "Marco Rossi" e "marco  rossi" sono lo stesso contatto. Nessun dato vocale
//! qui: i profili vocali restano in `voices/`.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;
use crate::store::{Store, normalize, now_ms};

pub const MAX_NAME_CHARS: usize = 60;
pub const MAX_ROLE_CHARS: usize = 100;
pub const MAX_DESCRIPTION_CHARS: usize = 1000;
pub const MAX_EMAIL_CHARS: usize = 254;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub id: i64,
    pub name: String,
    pub role: String,
    pub description: String,
    pub email: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Scheda da salvare. Senza `id` si crea, o si aggiorna il contatto con lo
/// stesso nome (la scheda del parlante "salva in rubrica" per nome).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContactInput {
    pub id: Option<i64>,
    pub name: String,
    pub role: String,
    pub description: String,
    pub email: String,
}

impl Store {
    /// Contatti in ordine alfabetico.
    pub fn contacts(&self) -> Result<Vec<Contact>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, role, description, email, created_at_ms, updated_at_ms
             FROM contacts ORDER BY name_key, id",
        )?;
        let rows = stmt.query_map([], contact_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn contact(&self, id: i64) -> Result<Contact, StoreError> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, name, role, description, email, created_at_ms, updated_at_ms
             FROM contacts WHERE id = ?1",
            params![id],
            contact_from_row,
        )
        .optional()?
        .ok_or(StoreError::ContactNotFound(id))
    }

    /// Crea o aggiorna una scheda (vedi [`ContactInput`]).
    pub fn save_contact(&self, input: &ContactInput) -> Result<Contact, StoreError> {
        let name = clip(&collapse(&input.name), MAX_NAME_CHARS);
        if name.is_empty() {
            return Err(StoreError::EmptyContactName);
        }
        let key = name_key(&name);
        let role = clip(&collapse(&input.role), MAX_ROLE_CHARS);
        let description = clip(input.description.trim(), MAX_DESCRIPTION_CHARS);
        let email = input.email.trim().to_owned();
        if !email.is_empty() && !is_valid_email(&email) {
            return Err(StoreError::InvalidEmail(email));
        }
        let now = now_ms();

        let conn = self.conn();
        let same_name: Option<i64> = conn
            .query_row(
                "SELECT id FROM contacts WHERE name_key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        let target = match (input.id, same_name) {
            (Some(id), Some(other)) if other != id => {
                return Err(StoreError::DuplicateContact(name));
            }
            (Some(id), _) => Some(id),
            (None, existing) => existing,
        };
        let id = match target {
            Some(id) => {
                // Unito per nome: resta la grafia già in rubrica.
                let name = if input.id.is_none() {
                    conn.query_row(
                        "SELECT name FROM contacts WHERE id = ?1",
                        params![id],
                        |row| row.get::<_, String>(0),
                    )?
                } else {
                    name
                };
                let changed = conn.execute(
                    "UPDATE contacts
                     SET name = ?1, name_key = ?2, role = ?3, description = ?4,
                         email = ?5, updated_at_ms = ?6
                     WHERE id = ?7",
                    params![name, key, role, description, email, now, id],
                )?;
                if changed == 0 {
                    return Err(StoreError::ContactNotFound(id));
                }
                id
            }
            None => {
                conn.execute(
                    "INSERT INTO contacts
                         (name, name_key, role, description, email, created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                    params![name, key, role, description, email, now],
                )?;
                conn.last_insert_rowid()
            }
        };
        drop(conn);
        self.contact(id)
    }

    pub fn delete_contact(&self, id: i64) -> Result<(), StoreError> {
        let conn = self.conn();
        if conn.execute("DELETE FROM contacts WHERE id = ?1", params![id])? == 0 {
            return Err(StoreError::ContactNotFound(id));
        }
        Ok(())
    }

    /// Nome di un parlante di "Loro" in un meeting salvato; vuoto = torna
    /// "Speaker N".
    pub fn set_speaker_name(
        &self,
        meeting_id: i64,
        speaker: u8,
        name: &str,
    ) -> Result<(), StoreError> {
        let name = clip(&collapse(name), MAX_NAME_CHARS);
        let conn = self.conn();
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM meetings WHERE id = ?1",
                params![meeting_id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !exists {
            return Err(StoreError::MeetingNotFound(meeting_id));
        }
        if name.is_empty() {
            conn.execute(
                "DELETE FROM speakers WHERE meeting_id = ?1 AND speaker = ?2",
                params![meeting_id, i64::from(speaker)],
            )?;
        } else {
            conn.execute(
                "INSERT INTO speakers (meeting_id, speaker, name) VALUES (?1, ?2, ?3)
                 ON CONFLICT(meeting_id, speaker) DO UPDATE SET name = excluded.name",
                params![meeting_id, i64::from(speaker), name],
            )?;
        }
        Ok(())
    }
}

/// Chiave di confronto dei nomi: minuscole, senza accenti, spazi compattati.
pub fn name_key(name: &str) -> String {
    collapse(&normalize(name))
}

/// Controllo sintattico essenziale (`locale@dominio.tld`, niente spazi né
/// caratteri che romperebbero le intestazioni): l'indirizzo vero lo verifica
/// il server di posta.
pub fn is_valid_email(email: &str) -> bool {
    if email.len() > MAX_EMAIL_CHARS {
        return false;
    }
    if email
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || "<>()[],;:\"\\".contains(c))
    {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect::<String>().trim().to_owned()
}

fn contact_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Contact> {
    Ok(Contact {
        id: row.get(0)?,
        name: row.get(1)?,
        role: row.get(2)?,
        description: row.get(3)?,
        email: row.get(4)?,
        created_at_ms: row.get(5)?,
        updated_at_ms: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("sottovoce.db")).unwrap();
        (dir, store)
    }

    fn input(name: &str, email: &str) -> ContactInput {
        ContactInput {
            name: name.into(),
            email: email.into(),
            ..ContactInput::default()
        }
    }

    #[test]
    fn creates_updates_by_name_and_deletes() {
        let (_dir, store) = store();
        let anna = store.save_contact(&input("  Anna   Bianchi ", "")).unwrap();
        assert_eq!(anna.name, "Anna Bianchi");

        // Stesso nome (maiuscole/accenti diversi) senza id = stessa scheda.
        let again = store
            .save_contact(&ContactInput {
                role: "CTO".into(),
                ..input("anna bianchi", "anna@example.com")
            })
            .unwrap();
        assert_eq!(again.id, anna.id);
        assert_eq!(again.role, "CTO");
        assert_eq!(again.email, "anna@example.com");
        assert_eq!(store.contacts().unwrap().len(), 1);

        let marco = store.save_contact(&input("Marco", "")).unwrap();
        // Rinominare Marco come Anna: duplicato.
        assert!(matches!(
            store.save_contact(&ContactInput {
                id: Some(marco.id),
                ..input("Anna Bianchi", "")
            }),
            Err(StoreError::DuplicateContact(_))
        ));
        let names: Vec<_> = store
            .contacts()
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["Anna Bianchi", "Marco"]);

        store.delete_contact(marco.id).unwrap();
        assert!(matches!(
            store.delete_contact(marco.id),
            Err(StoreError::ContactNotFound(_))
        ));
    }

    #[test]
    fn rejects_empty_name_and_bad_email() {
        let (_dir, store) = store();
        assert!(matches!(
            store.save_contact(&input("   ", "")),
            Err(StoreError::EmptyContactName)
        ));
        assert!(matches!(
            store.save_contact(&input("Anna", "anna@")),
            Err(StoreError::InvalidEmail(_))
        ));
        assert!(store.contacts().unwrap().is_empty());
    }

    #[test]
    fn email_syntax() {
        for ok in ["a@b.it", "nome.cognome+tag@sub.example.com"] {
            assert!(is_valid_email(ok), "{ok}");
        }
        for bad in [
            "",
            "a",
            "a@b",
            "@b.it",
            "a@.it",
            "a@b.",
            "a b@c.it",
            "a@b@c.it",
            "a@b..it",
            "<a@b.it>",
            "a@b.it\r\nBcc: x@y.z",
        ] {
            assert!(!is_valid_email(bad), "{bad}");
        }
    }

    #[test]
    fn names_speakers_of_saved_meetings() {
        let (dir, store) = store();
        let conn = rusqlite::Connection::open(dir.path().join("sottovoce.db")).unwrap();
        conn.execute(
            "INSERT INTO meetings (name, started_at_ms) VALUES ('m', 0)",
            [],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        drop(conn);

        store.set_speaker_name(id, 1, " Anna ").unwrap();
        store.set_speaker_name(id, 1, "Anna Bianchi").unwrap();
        store.set_speaker_name(id, 0, "Marco").unwrap();
        let names: Vec<_> = store
            .speakers(id)
            .unwrap()
            .into_iter()
            .map(|s| (s.speaker, s.name))
            .collect();
        assert_eq!(names, [(0, "Marco".into()), (1, "Anna Bianchi".into())]);
        store.set_speaker_name(id, 0, "  ").unwrap();
        assert_eq!(store.speakers(id).unwrap().len(), 1);
        assert!(matches!(
            store.set_speaker_name(999, 0, "x"),
            Err(StoreError::MeetingNotFound(999))
        ));
    }
}
