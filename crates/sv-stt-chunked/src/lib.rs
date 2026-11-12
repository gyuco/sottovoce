//! Motori STT cloud senza streaming, per Sottovoce.
//!
//! Pipeline per canale: `AudioFrame` → VAD locale (`sv-vad`, Silero) →
//! blocco di parlato in WAV → una richiesta HTTP → `SttEvent::Final`. Serve
//! ai servizi con API solo a file (Groq Whisper) o con streaming non adatto
//! (Google: gRPC con service account; Azure: WebSocket non documentato).
//! Rispetto allo streaming: niente parziali e un po' più di latenza.

mod azure;
mod engine;
mod google;
mod groq;

use std::sync::Arc;
use std::time::Duration;

pub use azure::{Azure, AzureConfig};
pub use engine::{Batching, ChunkedEngine};
pub use google::{Google, GoogleConfig};
pub use groq::{DEFAULT_MODEL as GROQ_MODEL, Groq, GroqConfig};
use sv_stt::Language;

/// Esito della trascrizione di un blocco.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    /// Vuoto = nessun parlato riconosciuto.
    pub text: String,
    /// Lingua rilevata dal servizio (codice a due lettere), se la dichiara.
    pub language: Option<String>,
    pub confidence: Option<f32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    #[error("API key non valida o non autorizzata")]
    Unauthorized,
    /// Configurazione o richiesta rifiutata: inutile ritentare.
    #[error("{0}")]
    Fatal(String),
    /// Troppe richieste: ritentare dopo l'attesa indicata.
    #[error("limite di richieste raggiunto")]
    RateLimited(Option<Duration>),
    /// Rete o errore del server: si può ritentare.
    #[error("{0}")]
    Temporary(String),
}

/// Servizio che trascrive un blocco di audio 16 kHz mono.
pub trait Transcriber: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    /// Come raggruppare le frasi in richieste.
    fn batching(&self) -> Batching {
        Batching::PER_SENTENCE
    }

    fn transcribe(
        &self,
        samples: Arc<[f32]>,
        language: Language,
    ) -> impl Future<Output = Result<Transcript, ChunkError>> + Send;
}

/// WAV PCM s16le 16 kHz mono (header di 44 byte).
pub(crate) fn wav(samples: &[f32]) -> Vec<u8> {
    let pcm = sv_stt::pcm16(samples);
    let rate = sv_audio::TARGET_SAMPLE_RATE;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // byte/s
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bit
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(&pcm);
    out
}

/// Client HTTP condiviso (rustls + ring, certificati di sistema).
pub(crate) fn http_client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(concat!("sottovoce/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

/// Invia e legge il corpo; errori di rete = temporanei.
pub(crate) async fn send(
    req: reqwest::RequestBuilder,
) -> Result<(u16, String, Option<Duration>), ChunkError> {
    let resp = req
        .send()
        .await
        .map_err(|e| ChunkError::Temporary(e.to_string()))?;
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|s| Duration::from_secs_f64(s.max(0.0)));
    let body = resp
        .text()
        .await
        .map_err(|e| ChunkError::Temporary(e.to_string()))?;
    Ok((status, body, retry_after))
}

/// Stato HTTP di errore → [`ChunkError`] (401/403 = chiave).
pub(crate) fn status_error(
    service: &str,
    status: u16,
    body: &str,
    retry_after: Option<Duration>,
) -> ChunkError {
    match status {
        401 | 403 => ChunkError::Unauthorized,
        429 => ChunkError::RateLimited(retry_after),
        408 | 500..=599 => ChunkError::Temporary(format!("{service}: HTTP {status}")),
        _ => ChunkError::Fatal(format!("{service}: HTTP {status}: {}", short(body))),
    }
}

/// Corpo di errore accorciato per i messaggi.
pub(crate) fn short(body: &str) -> String {
    let body = body.trim();
    match body.char_indices().nth(300) {
        Some((i, _)) => format!("{}…", &body[..i]),
        None => body.to_owned(),
    }
}

/// Locale BCP-47 per i servizi che vogliono la regione.
pub(crate) fn locale(language: Language) -> &'static str {
    match language {
        Language::It | Language::Multi => "it-IT",
        Language::En => "en-US",
        Language::Es => "es-ES",
        Language::Fr => "fr-FR",
        Language::De => "de-DE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header() {
        let w = wav(&[0.0, 1.0]);
        assert_eq!(w.len(), 48);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()), 40);
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 4);
        assert_eq!(&w[46..48], &[0xff, 0x7f]);
    }

    #[test]
    fn status_mapping() {
        assert!(matches!(
            status_error("x", 401, "", None),
            ChunkError::Unauthorized
        ));
        assert!(matches!(
            status_error("x", 429, "", Some(Duration::from_secs(3))),
            ChunkError::RateLimited(Some(d)) if d == Duration::from_secs(3)
        ));
        assert!(matches!(
            status_error("x", 503, "", None),
            ChunkError::Temporary(_)
        ));
        assert!(
            matches!(status_error("x", 400, "bad", None), ChunkError::Fatal(m) if m.contains("bad"))
        );
    }
}
