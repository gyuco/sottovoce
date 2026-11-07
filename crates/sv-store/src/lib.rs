//! Persistenza SQLite dei meeting di Sottovoce.
//!
//! - [`Store`]: apertura/migrazione del DB e lettura (storico, dettaglio
//!   meeting), più il recupero dei meeting rimasti aperti all'avvio.
//! - [`Writer`]/[`StoreHandle`]: scrittura da un thread dedicato, a blocchi in
//!   transazione, senza mai bloccare il pump STT.
//!
//! Scelta del driver: `rusqlite` con `bundled` (SQLite compilato insieme
//! all'app). Niente server, niente runtime async nel crate: una connessione
//! per lo scrittore e una per le letture, WAL per non fermare le letture
//! durante le scritture.

mod error;
mod schema;
mod store;
mod types;
mod writer;

pub use error::StoreError;
pub use schema::SCHEMA_VERSION;
pub use store::Store;
pub use types::{
    Assist, Meeting, MeetingDetails, MeetingDraft, MeetingPage, MeetingSummary, NewSegment,
    Segment, SpeakerName, Translation,
};
pub use writer::{FLUSH_INTERVAL, QUEUE, StoreHandle, Writer};
