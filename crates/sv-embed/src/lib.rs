//! Embedding di testo locali per la ricerca semantica di Sottovoce.
//!
//! Modello: Google EmbeddingGemma 2 (solo il ramo testo, quantizzato q4) in
//! ONNX. Il runtime è l'ONNX Runtime già collegato staticamente da
//! `sherpa-onnx`: `ort` ne usa l'API invece di portarne una seconda copia.
//!
//! - [`Embedder`]: tokenizza, esegue il modello a lotti, restituisce vettori
//!   normalizzati di [`DIM`] dimensioni (troncamento Matryoshka).
//! - [`query_prompt`] / [`document_prompt`]: i prefissi richiesti dal modello
//!   per la ricerca asimmetrica.
//! - [`QuantVec`]: vettore in int8 con scala, per l'indice su disco.

mod error;
mod quant;

use std::path::Path;
use std::sync::Once;

use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;

pub use error::EmbedError;
pub use quant::QuantVec;

// Assicura che la libreria statica di sherpa-onnx (e con lei ONNX Runtime) sia
// collegata anche se nessun altro crate del binario la usa.
use sherpa_onnx as _;

/// Dimensione dei vettori prodotti. EmbeddingGemma 2 è addestrato con
/// Matryoshka: i primi 512 valori dei 768 nativi, rinormalizzati, perdono
/// circa 0,2 punti su MTEB multilingue.
pub const DIM: usize = 512;
/// Dimensione nativa dell'uscita `sentence_embedding`.
const NATIVE_DIM: usize = 768;
/// Token massimi per testo (BOS ed EOS inclusi). I blocchi di trascrizione
/// stanno largamente sotto; oltre si tronca.
pub const MAX_TOKENS: usize = 512;
/// Testi per esecuzione del modello.
const BATCH: usize = 8;
/// Dimensione delle feature multimodali in ingresso (vuote per il solo testo).
const MEDIA_DIM: usize = 512;

/// File del modello dentro la sua cartella.
pub const MODEL_FILE: &str = "model_q4.onnx";
pub const TOKENIZER_FILE: &str = "tokenizer.json";

/// Prompt per una query di ricerca.
pub fn query_prompt(query: &str) -> String {
    format!("task: search result | query: {}", query.trim())
}

/// Prompt per un documento dell'indice. `title` vuoto o assente = `none`.
pub fn document_prompt(title: Option<&str>, text: &str) -> String {
    let title = title
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "none".to_owned());
    format!("title: {title} | text: {}", text.trim())
}

/// Tronca un embedding a `dim` valori e lo rinormalizza (norma 1).
pub fn truncate(v: &[f32], dim: usize) -> Vec<f32> {
    let mut out: Vec<f32> = v.iter().take(dim).copied().collect();
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        out.iter_mut().for_each(|x| *x /= norm);
    }
    out
}

/// Collega `ort` all'ONNX Runtime presente nel binario. Idempotente.
fn init_runtime() {
    unsafe extern "C" {
        fn OrtGetApiBase() -> *const ort::sys::OrtApiBase;
    }
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // SAFETY: `OrtGetApiBase` è esportata dalla libreria statica di ONNX
        // Runtime collegata da sherpa-onnx. `GetApi` restituisce un puntatore
        // a una tabella statica di funzioni, o null se la versione richiesta
        // non è supportata: lo controlliamo prima di copiarla.
        let api = unsafe {
            let base = OrtGetApiBase();
            assert!(!base.is_null(), "ONNX Runtime: OrtGetApiBase null");
            let api = ((*base).GetApi)(ort::sys::ORT_API_VERSION);
            assert!(!api.is_null(), "ONNX Runtime: versione API non supportata");
            std::ptr::read(api)
        };
        ort::set_api(api);
    });
}

/// Modello di embedding caricato. Non è `Sync`: va usato da un solo thread
/// alla volta (l'indicizzatore lo tiene dietro un `Mutex`).
pub struct Embedder {
    session: Session,
    tokenizer: Tokenizer,
}

impl Embedder {
    /// Carica modello e tokenizer da `dir`. `threads` = thread di calcolo
    /// (pochi: l'indicizzazione gira in background).
    pub fn open(dir: &Path, threads: usize) -> Result<Self, EmbedError> {
        init_runtime();
        let tokenizer = Tokenizer::from_file(dir.join(TOKENIZER_FILE))
            .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;
        let model = |e: String| EmbedError::Model(e);
        let session = Session::builder()
            .map_err(|e| model(e.to_string()))?
            .with_intra_threads(threads.max(1))
            .map_err(|e| model(e.to_string()))?
            .commit_from_file(dir.join(MODEL_FILE))
            .map_err(|e| model(e.to_string()))?;
        Ok(Self { session, tokenizer })
    }

    /// Embedding di una query (prefisso di ricerca incluso).
    pub fn embed_query(&mut self, query: &str) -> Result<Vec<f32>, EmbedError> {
        let mut out = self.embed_raw(&[query_prompt(query)])?;
        Ok(out.pop().unwrap_or_default())
    }

    /// Embedding di documenti già formattati con [`document_prompt`],
    /// nello stesso ordine.
    pub fn embed_documents(&mut self, docs: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.embed_raw(docs)
    }

    fn embed_raw(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let mut ids = Vec::with_capacity(texts.len());
        for text in texts {
            let enc = self
                .tokenizer
                .encode(text.as_str(), true)
                .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;
            ids.push(clip(enc.get_ids()));
        }
        // Lotti di lunghezza simile: meno padding.
        let mut order: Vec<usize> = (0..ids.len()).collect();
        order.sort_by_key(|&i| ids[i].len());
        let mut out = vec![Vec::new(); ids.len()];
        for chunk in order.chunks(BATCH) {
            let batch: Vec<&[i64]> = chunk.iter().map(|&i| ids[i].as_slice()).collect();
            for (&i, v) in chunk.iter().zip(self.run(&batch)?) {
                out[i] = v;
            }
        }
        Ok(out)
    }

    fn run(&mut self, batch: &[&[i64]]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let (input_ids, mask, shape) = pad(batch);
        let model = |e: ort::Error| EmbedError::Model(e.to_string());
        let empty = || Tensor::from_array(([0usize, MEDIA_DIM], Vec::<f32>::new()));
        let outputs = self
            .session
            .run(ort::inputs! {
                "input_ids" => Tensor::from_array((shape, input_ids)).map_err(model)?,
                "attention_mask" => Tensor::from_array((shape, mask)).map_err(model)?,
                "image_features" => empty().map_err(model)?,
                "video_features" => empty().map_err(model)?,
                "audio_features" => empty().map_err(model)?,
            })
            .map_err(model)?;
        let (dims, data) = outputs["sentence_embedding"]
            .try_extract_tensor::<f32>()
            .map_err(model)?;
        if dims.len() != 2 || dims[0] as usize != batch.len() || dims[1] as usize != NATIVE_DIM {
            return Err(EmbedError::Model(format!("forma inattesa {dims:?}")));
        }
        Ok(data
            .chunks_exact(NATIVE_DIM)
            .map(|v| truncate(v, DIM))
            .collect())
    }
}

/// Converte gli id e tronca a [`MAX_TOKENS`] tenendo l'ultimo token (EOS).
fn clip(ids: &[u32]) -> Vec<i64> {
    let mut out: Vec<i64> = ids.iter().map(|&x| i64::from(x)).collect();
    if out.len() > MAX_TOKENS {
        let last = out[out.len() - 1];
        out.truncate(MAX_TOKENS - 1);
        out.push(last);
    }
    out
}

/// Padding a destra con id 0 e maschera 0. Restituisce (id, maschera, forma).
fn pad(batch: &[&[i64]]) -> (Vec<i64>, Vec<i64>, [usize; 2]) {
    let len = batch.iter().map(|b| b.len()).max().unwrap_or(0);
    let mut ids = Vec::with_capacity(batch.len() * len);
    let mut mask = Vec::with_capacity(batch.len() * len);
    for row in batch {
        ids.extend_from_slice(row);
        ids.extend(std::iter::repeat_n(0, len - row.len()));
        mask.extend(std::iter::repeat_n(1, row.len()));
        mask.extend(std::iter::repeat_n(0, len - row.len()));
    }
    (ids, mask, [batch.len(), len])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_follow_the_model_card() {
        assert_eq!(
            query_prompt("  costi hosting "),
            "task: search result | query: costi hosting"
        );
        assert_eq!(document_prompt(None, "ciao"), "title: none | text: ciao");
        assert_eq!(
            document_prompt(Some("  "), "ciao"),
            "title: none | text: ciao"
        );
        assert_eq!(
            document_prompt(Some("Sprint\n review"), "ok"),
            "title: Sprint review | text: ok"
        );
    }

    #[test]
    fn truncate_renormalizes() {
        let v = [3.0, 4.0, 100.0];
        let t = truncate(&v, 2);
        assert_eq!(t.len(), 2);
        assert!((t[0] - 0.6).abs() < 1e-6 && (t[1] - 0.8).abs() < 1e-6);
        assert_eq!(truncate(&[0.0, 0.0], 2), vec![0.0, 0.0]);
    }

    #[test]
    fn clip_keeps_the_last_token() {
        let ids: Vec<u32> = (0..(MAX_TOKENS as u32 + 10)).collect();
        let out = clip(&ids);
        assert_eq!(out.len(), MAX_TOKENS);
        assert_eq!(out[MAX_TOKENS - 1], i64::from(*ids.last().unwrap()));
        assert_eq!(clip(&[2, 5, 1]), vec![2, 5, 1]);
    }

    #[test]
    fn pad_right_with_mask() {
        let (ids, mask, shape) = pad(&[&[2, 7, 1], &[2, 1]]);
        assert_eq!(shape, [2, 3]);
        assert_eq!(ids, vec![2, 7, 1, 2, 1, 0]);
        assert_eq!(mask, vec![1, 1, 1, 1, 1, 0]);
    }
}
