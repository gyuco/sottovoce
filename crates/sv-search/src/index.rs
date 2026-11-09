//! Indici dei meeting su disco più una cache in memoria per la ricerca.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use sv_embed::QuantVec;
use sv_store::MeetingDetails;

use crate::chunk::{self, Chunk, ChunkRef};
use crate::file::{Entry, IndexFile};

/// Lavoro da fare per indicizzare un meeting: blocchi, impronta e prompt
/// pronti per il modello (stesso ordine).
pub struct Prepared {
    pub chunks: Vec<Chunk>,
    pub fingerprint: u64,
    pub prompts: Vec<String>,
}

/// Blocchi e prompt di un meeting. Il titolo del documento è il nome del
/// meeting, come suggerito dalla scheda del modello.
pub fn prepare(details: &MeetingDetails) -> Prepared {
    let chunks = chunk::chunks(details);
    let title = details.meeting.name.trim();
    let fingerprint = chunk::fingerprint(title, &chunks);
    let prompts = chunks
        .iter()
        .map(|c| sv_embed::document_prompt(Some(title), &c.text))
        .collect();
    Prepared {
        chunks,
        fingerprint,
        prompts,
    }
}

/// Passaggio trovato dalla ricerca semantica.
#[derive(Debug, Clone, PartialEq)]
pub struct PassageHit {
    pub meeting_id: i64,
    pub source: ChunkRef,
    pub start_ms: u64,
    /// Similarità coseno con la query.
    pub score: f32,
}

pub struct SemanticIndex {
    dir: PathBuf,
    model: String,
    dim: usize,
    /// Indici validi per il modello corrente, caricati alla prima ricerca.
    cache: Mutex<Option<HashMap<i64, Arc<IndexFile>>>>,
}

impl SemanticIndex {
    /// `dir` = cartella degli indici (`app_data_dir/index`).
    pub fn new(dir: impl Into<PathBuf>, model: impl Into<String>, dim: usize) -> Self {
        Self {
            dir: dir.into(),
            model: model.into(),
            dim,
            cache: Mutex::new(None),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, meeting_id: i64) -> PathBuf {
        self.dir.join(format!("{meeting_id}.vec"))
    }

    fn usable(&self, file: &IndexFile) -> bool {
        file.model == self.model && file.dim == self.dim
    }

    /// Cache caricata (legge tutta la cartella al primo accesso).
    fn cache(&self) -> MutexGuard<'_, Option<HashMap<i64, Arc<IndexFile>>>> {
        let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(self.load_all());
        }
        guard
    }

    fn load_all(&self) -> HashMap<i64, Arc<IndexFile>> {
        let mut map = HashMap::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) => {
                if e.kind() != io::ErrorKind::NotFound {
                    tracing::warn!(dir = %self.dir.display(), "cartella indici illeggibile: {e}");
                }
                return map;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("vec") {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<i64>().ok())
            else {
                continue;
            };
            match IndexFile::read(&path) {
                Ok(Some(file)) if self.usable(&file) => {
                    map.insert(id, Arc::new(file));
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(path = %path.display(), "indice illeggibile: {e}"),
            }
        }
        map
    }

    /// `true` se l'indice del meeting esiste, è del modello corrente e
    /// corrisponde a questa impronta del contenuto.
    pub fn is_fresh(&self, meeting_id: i64, fingerprint: u64) -> bool {
        self.cache()
            .as_ref()
            .and_then(|m| m.get(&meeting_id))
            .is_some_and(|f| f.fingerprint == fingerprint)
    }

    /// Meeting con un indice valido.
    pub fn indexed(&self) -> Vec<i64> {
        let mut ids: Vec<i64> = self
            .cache()
            .as_ref()
            .map(|m| m.keys().copied().collect())
            .unwrap_or_default();
        ids.sort_unstable();
        ids
    }

    /// Salva l'indice di un meeting a partire dai vettori dei blocchi di
    /// `prepared` (stesso ordine). Un meeting senza blocchi non lascia file.
    pub fn store(
        &self,
        meeting_id: i64,
        prepared: &Prepared,
        vectors: &[Vec<f32>],
    ) -> io::Result<()> {
        if prepared.chunks.len() != vectors.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "blocchi e vettori non corrispondono",
            ));
        }
        if prepared.chunks.is_empty() {
            return self.remove(meeting_id);
        }
        let file = IndexFile {
            model: self.model.clone(),
            dim: self.dim,
            fingerprint: prepared.fingerprint,
            entries: prepared
                .chunks
                .iter()
                .zip(vectors)
                .map(|(c, v)| Entry {
                    source: c.source,
                    start_ms: c.start_ms,
                    vector: QuantVec::quantize(&sv_embed::truncate(v, self.dim)),
                })
                .collect(),
        };
        file.write(&self.path(meeting_id))?;
        if let Some(map) = self.cache().as_mut() {
            map.insert(meeting_id, Arc::new(file));
        }
        Ok(())
    }

    /// Elimina l'indice di un meeting (anche se non c'è).
    pub fn remove(&self, meeting_id: i64) -> io::Result<()> {
        if let Some(map) = self.cache().as_mut() {
            map.remove(&meeting_id);
        }
        match std::fs::remove_file(self.path(meeting_id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Elimina gli indici dei meeting che non esistono più.
    pub fn retain(&self, existing: &[i64]) {
        for id in self.indexed() {
            if !existing.contains(&id)
                && let Err(e) = self.remove(id)
            {
                tracing::warn!(meeting = id, "indice orfano non eliminato: {e}");
            }
        }
    }

    /// Passaggi più simili alla query (vettore normalizzato di `dim` valori),
    /// al massimo `limit`, scartando quelli sotto `best - margin`: i punteggi
    /// assoluti variano molto da query a query, la distanza dal migliore no.
    pub fn search(&self, query: &[f32], limit: usize, margin: f32) -> Vec<PassageHit> {
        if query.len() != self.dim {
            return Vec::new();
        }
        let files: Vec<(i64, Arc<IndexFile>)> = self
            .cache()
            .as_ref()
            .map(|m| m.iter().map(|(id, f)| (*id, f.clone())).collect())
            .unwrap_or_default();
        let mut hits: Vec<PassageHit> = files
            .iter()
            .flat_map(|(id, file)| {
                file.entries.iter().map(move |e| PassageHit {
                    meeting_id: *id,
                    source: e.source,
                    start_ms: e.start_ms,
                    score: e.vector.dot(query),
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        let Some(best) = hits.first().map(|h| h.score) else {
            return hits;
        };
        hits.retain(|h| h.score >= best - margin);
        hits.truncate(limit);
        hits
    }
}

/// Testo di un passaggio, ricostruito dal meeting com'è ora. `None` se il
/// meeting è cambiato e il blocco non esiste più (l'indice verrà rifatto).
pub fn passage_text(details: &MeetingDetails, source: ChunkRef) -> Option<String> {
    chunk::chunks(details)
        .into_iter()
        .find(|c| c.source == source)
        .map(|c| c.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::tests::{details, segment};

    fn unit(dim: usize, i: usize) -> Vec<f32> {
        let mut v = vec![0.0; dim];
        v[i] = 1.0;
        v
    }

    fn prepared_for(id: i64) -> (MeetingDetails, Prepared) {
        let mut d = details(vec![
            segment(id * 10 + 1, "me", None, 0, "uno"),
            segment(id * 10 + 2, "them", Some(0), 1000, "due"),
        ]);
        d.meeting.id = id;
        let p = prepare(&d);
        (d, p)
    }

    #[test]
    fn prompts_carry_the_meeting_title() {
        let (_, p) = prepared_for(1);
        assert_eq!(
            p.prompts,
            vec!["title: Sprint review | text: Io: uno\nAnna: due"]
        );
    }

    #[test]
    fn store_search_remove() {
        let dir = tempfile::tempdir().unwrap();
        let index = SemanticIndex::new(dir.path().join("index"), "m", 4);
        assert!(index.indexed().is_empty());

        let (_, p1) = prepared_for(1);
        let (_, p2) = prepared_for(2);
        index.store(1, &p1, &[unit(4, 0)]).unwrap();
        index.store(2, &p2, &[unit(4, 1)]).unwrap();
        assert_eq!(index.indexed(), vec![1, 2]);
        assert!(index.is_fresh(1, p1.fingerprint));
        assert!(!index.is_fresh(1, p1.fingerprint + 1));

        let q = sv_embed::truncate(&[0.9, 0.3, 0.0, 0.0], 4);
        let hits = index.search(&q, 10, 1.0);
        assert_eq!(
            hits.iter().map(|h| h.meeting_id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(hits[0].score > 0.9);
        // Il margine scarta i passaggi lontani dal migliore.
        assert_eq!(index.search(&q, 10, 0.2).len(), 1);
        // Dimensione sbagliata: nessun risultato invece di un panic.
        assert!(index.search(&[1.0], 10, 1.0).is_empty());

        // Una nuova istanza rilegge i file dal disco.
        let reopened = SemanticIndex::new(dir.path().join("index"), "m", 4);
        assert_eq!(reopened.indexed(), vec![1, 2]);
        // Un altro modello non usa questi file.
        let other = SemanticIndex::new(dir.path().join("index"), "altro", 4);
        assert!(other.indexed().is_empty());

        index.remove(1).unwrap();
        assert!(!dir.path().join("index/1.vec").exists());
        assert_eq!(index.indexed(), vec![2]);
        index.retain(&[]);
        assert!(index.indexed().is_empty());
        assert!(!dir.path().join("index/2.vec").exists());
    }

    #[test]
    fn corrupt_file_is_not_fresh() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("index")).unwrap();
        std::fs::write(dir.path().join("index/1.vec"), b"rotto").unwrap();
        let index = SemanticIndex::new(dir.path().join("index"), "m", 4);
        let (_, p) = prepared_for(1);
        assert!(!index.is_fresh(1, p.fingerprint));
        index.store(1, &p, &[unit(4, 2)]).unwrap();
        assert!(index.is_fresh(1, p.fingerprint));
    }

    #[test]
    fn mismatched_vectors_are_refused_and_empty_meetings_leave_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let index = SemanticIndex::new(dir.path(), "m", 4);
        let (_, p) = prepared_for(1);
        assert!(index.store(1, &p, &[]).is_err());
        let empty = prepare(&details(Vec::new()));
        index.store(1, &empty, &[]).unwrap();
        assert!(!dir.path().join("1.vec").exists());
    }

    #[test]
    fn passage_text_rebuilds_the_chunk() {
        let (d, p) = prepared_for(3);
        assert_eq!(
            passage_text(&d, p.chunks[0].source).as_deref(),
            Some("Io: uno\nAnna: due")
        );
        assert_eq!(passage_text(&d, ChunkRef::Overview), None);
    }
}
