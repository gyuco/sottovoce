//! File dell'indice di un meeting: `index/<meeting_id>.vec`.
//!
//! Binario little-endian:
//!
//! ```text
//! "SVIX" | versione u16 | dim u16 | len u8 + id modello | impronta u64 | n u32
//! n × ( tipo u8 | a i64 | b i64 | start_ms u64 | scala f32 | dim × i8 )
//! ```
//!
//! Niente testo: per mostrare un risultato si rilegge il meeting dal DB e dal
//! file di trascrizione. I vettori però permettono di ricostruire in parte il
//! testo, quindi vanno trattati come la trascrizione (stessa cartella dati,
//! eliminati insieme al meeting).

use std::io::{self, Write};
use std::path::Path;

use sv_embed::QuantVec;

use crate::chunk::ChunkRef;

const MAGIC: &[u8; 4] = b"SVIX";
const VERSION: u16 = 1;
/// Byte fissi di ogni blocco prima del vettore: tipo, a, b, start_ms, scala.
const ENTRY_HEAD: usize = 1 + 8 + 8 + 8 + 4;

/// Un blocco indicizzato.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub source: ChunkRef,
    pub start_ms: u64,
    pub vector: QuantVec,
}

/// Indice di un meeting.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexFile {
    pub model: String,
    pub dim: usize,
    pub fingerprint: u64,
    pub entries: Vec<Entry>,
}

impl IndexFile {
    pub fn to_bytes(&self) -> Vec<u8> {
        let model = self.model.as_bytes();
        let model_len = model.len().min(u8::MAX as usize);
        let mut out = Vec::with_capacity(32 + self.entries.len() * (ENTRY_HEAD + self.dim));
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.dim as u16).to_le_bytes());
        out.push(model_len as u8);
        out.extend_from_slice(&model[..model_len]);
        out.extend_from_slice(&self.fingerprint.to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for e in &self.entries {
            let (kind, a, b) = e.source.encode();
            out.push(kind);
            out.extend_from_slice(&a.to_le_bytes());
            out.extend_from_slice(&b.to_le_bytes());
            out.extend_from_slice(&e.start_ms.to_le_bytes());
            out.extend_from_slice(&e.vector.scale.to_le_bytes());
            out.extend(e.vector.data.iter().map(|&q| q as u8));
        }
        out
    }

    /// `None` se il file è troncato, di un'altra versione o incoerente.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader(bytes);
        if r.take(4)? != MAGIC || r.u16()? != VERSION {
            return None;
        }
        let dim = usize::from(r.u16()?);
        let model_len = usize::from(r.take(1)?[0]);
        let model = String::from_utf8(r.take(model_len)?.to_vec()).ok()?;
        let fingerprint = r.u64()?;
        let n = r.u32()? as usize;
        // Controllo della lunghezza prima di allocare.
        if r.0.len() != n.checked_mul(ENTRY_HEAD + dim)? {
            return None;
        }
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let kind = r.take(1)?[0];
            let a = r.i64()?;
            let b = r.i64()?;
            let start_ms = r.u64()?;
            let scale = f32::from_le_bytes(r.take(4)?.try_into().ok()?);
            let data = r.take(dim)?.iter().map(|&q| q as i8).collect();
            entries.push(Entry {
                source: ChunkRef::decode(kind, a, b)?,
                start_ms,
                vector: QuantVec { scale, data },
            });
        }
        Some(Self {
            model,
            dim,
            fingerprint,
            entries,
        })
    }

    /// Legge il file; `Ok(None)` se manca o non è valido.
    pub fn read(path: &Path) -> io::Result<Option<Self>> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let file = Self::from_bytes(&bytes);
                if file.is_none() {
                    tracing::warn!(path = %path.display(), "indice non valido, va rifatto");
                }
                Ok(file)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Scrittura atomica: file temporaneo nella stessa cartella + rename.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("vec.tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&self.to_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Some(head)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample() -> IndexFile {
        IndexFile {
            model: "embeddinggemma-2-q4".into(),
            dim: 4,
            fingerprint: 42,
            entries: vec![
                Entry {
                    source: ChunkRef::Transcript { first: 1, last: 6 },
                    start_ms: 1500,
                    vector: QuantVec {
                        scale: 0.01,
                        data: vec![1, -2, 127, -127],
                    },
                },
                Entry {
                    source: ChunkRef::Overview,
                    start_ms: 0,
                    vector: QuantVec {
                        scale: 0.5,
                        data: vec![0, 0, 3, 4],
                    },
                },
            ],
        }
    }

    #[test]
    fn round_trip() {
        let f = sample();
        assert_eq!(IndexFile::from_bytes(&f.to_bytes()), Some(f));
    }

    #[test]
    fn rejects_truncated_or_foreign_bytes() {
        let bytes = sample().to_bytes();
        assert_eq!(IndexFile::from_bytes(&bytes[..bytes.len() - 1]), None);
        assert_eq!(IndexFile::from_bytes(b"nope"), None);
        let mut extra = bytes.clone();
        extra.push(0);
        assert_eq!(IndexFile::from_bytes(&extra), None);
        let mut bad_kind = bytes;
        let first_entry = 4 + 2 + 2 + 1 + "embeddinggemma-2-q4".len() + 8 + 4;
        bad_kind[first_entry] = 99;
        assert_eq!(IndexFile::from_bytes(&bad_kind), None);
    }

    #[test]
    fn write_and_read_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index").join("7.vec");
        assert_eq!(IndexFile::read(&path).unwrap(), None);
        sample().write(&path).unwrap();
        assert_eq!(IndexFile::read(&path).unwrap(), Some(sample()));
        std::fs::write(&path, b"garbage").unwrap();
        assert_eq!(IndexFile::read(&path).unwrap(), None);
    }
}
