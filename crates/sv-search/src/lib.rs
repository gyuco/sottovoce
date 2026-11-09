//! Ricerca semantica nei meeting di Sottovoce.
//!
//! - [`chunks`]: divide un meeting in blocchi (trascrizione con etichette
//!   dei parlanti, compressione, riepilogo).
//! - [`SemanticIndex`]: un file `<meeting_id>.vec` per meeting con i vettori
//!   int8 dei blocchi (nessun testo), più una cache in memoria per la ricerca
//!   esaustiva. Un'impronta del contenuto dice quando l'indice è vecchio.
//! - [`fuse`]: Reciprocal Rank Fusion tra passaggi semantici e risultati
//!   della ricerca per parole.
//! - [`build_context`] e compagni: recupero e contesto numerato per le
//!   domande sullo storico (RAG).
//!
//! Il calcolo dei vettori (modello) resta fuori: il crate riceve i vettori
//! già fatti, così l'app decide thread, priorità e lotti.

mod chunk;
mod file;
mod fuse;
mod index;
mod rag;

pub use chunk::{Chunk, ChunkRef, MAX_SEGMENTS, TARGET_WORDS, chunks, fingerprint};
pub use file::{Entry, IndexFile};
pub use fuse::{MeetingHit, RRF_K, fuse};
pub use index::{PassageHit, Prepared, SemanticIndex, passage_text, prepare};
pub use rag::{
    CONTEXT_BUDGET, Candidate, Context, MAX_PASSAGES, PER_MEETING, Source, build_context,
    citations, date, fuse_passages, lexical_hits, query_terms, strip_citations,
};
