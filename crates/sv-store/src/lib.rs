//! Persistenza SQLite dei meeting di Sottovoce.
//!
//! - [`Store`]: apertura/migrazione del DB e lettura (storico, dettaglio
//!   meeting), più il recupero dei meeting rimasti aperti all'avvio.
//! - Rubrica ([`Contact`]): schede delle persone incontrate nei meeting,
//!   legate ai parlanti per nome.
//! - [`Writer`]/[`StoreHandle`]: scrittura da un thread dedicato, a blocchi in
//!   transazione, senza mai bloccare il pump STT.
//!
//! Il testo delle frasi e delle traduzioni non è nelle tabelle: vive in
//! `transcripts/<meeting_id>.txt` accanto al database, una riga per voce. Le
//! tabelle conservano solo il riferimento (file del meeting + numero di riga),
//! così il DB resta leggero e le trascrizioni sono testo semplice su disco.
//!
//! Scelta del driver: `rusqlite` con `bundled` (SQLite compilato insieme
//! all'app). Niente server, niente runtime async nel crate: una connessione
//! per lo scrittore e una per le letture, WAL per non fermare le letture
//! durante le scritture.

mod contacts;
mod error;
mod schema;
mod store;
mod transcript;
mod types;
mod writer;

pub use contacts::{Contact, ContactInput, is_valid_email, name_key};
pub use error::StoreError;
pub use schema::SCHEMA_VERSION;
pub use store::{Store, normalize};
pub use types::{
    ActionItem, Assist, CompressionBlock, ImportantMoment, KeyPoint, Meeting, MeetingDetails,
    MeetingDraft, MeetingPage, MeetingSummary, NewCompressionBlock, NewSegment, NewSummary,
    Segment, SpeakerName, Summary, Translation,
};
pub use writer::{FLUSH_INTERVAL, QUEUE, StoreHandle, Writer};
