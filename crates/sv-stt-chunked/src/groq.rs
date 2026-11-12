//! Groq: Whisper (large-v3-turbo) via API compatibile OpenAI, a file.
//!
//! Groq fattura almeno 10 s per richiesta e limita le richieste al minuto:
//! le frasi si uniscono in blocchi di ~8 s (attesa massima 10 s). Whisper
//! sul silenzio "inventa" frasi: si scartano i blocchi in cui tutti i
//! segmenti hanno alta probabilità di non-parlato.

use std::sync::Arc;
use std::time::Duration;

use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use sv_stt::Language;

use crate::{Batching, ChunkError, Transcriber, Transcript};

const ENDPOINT: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
pub const DEFAULT_MODEL: &str = "whisper-large-v3-turbo";
/// Sopra questa probabilità un segmento è considerato silenzio/rumore.
const NO_SPEECH: f32 = 0.6;

#[derive(Debug, Clone)]
pub struct GroqConfig {
    pub api_key: String,
    pub model: String,
}

pub struct Groq {
    config: GroqConfig,
    http: reqwest::Client,
}

impl Groq {
    pub fn new(config: GroqConfig) -> Result<Self, ChunkError> {
        if config.api_key.trim().is_empty() {
            return Err(ChunkError::Fatal("API key Groq mancante".into()));
        }
        Ok(Self {
            config,
            http: crate::http_client(),
        })
    }
}

#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default)]
    text: String,
    /// Nome inglese ("italian") o codice ("it"), secondo la versione.
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    segments: Vec<ResponseSegment>,
}

#[derive(Debug, Deserialize)]
struct ResponseSegment {
    #[serde(default)]
    no_speech_prob: f32,
}

impl Transcriber for Groq {
    fn name(&self) -> &'static str {
        "groq"
    }

    fn batching(&self) -> Batching {
        Batching {
            min_audio: Duration::from_secs(8),
            max_wait: Duration::from_secs(10),
            max_audio: Duration::from_secs(60),
        }
    }

    async fn transcribe(
        &self,
        samples: Arc<[f32]>,
        language: Language,
    ) -> Result<Transcript, ChunkError> {
        let file = Part::bytes(crate::wav(&samples))
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| ChunkError::Fatal(e.to_string()))?;
        let mut form = Form::new()
            .part("file", file)
            .text("model", self.config.model.clone())
            .text("response_format", "verbose_json")
            .text("temperature", "0");
        if language != Language::Multi {
            form = form.text("language", language.code());
        }
        let req = self
            .http
            .post(ENDPOINT)
            .bearer_auth(self.config.api_key.trim())
            .multipart(form);
        let (status, body, retry_after) = crate::send(req).await?;
        if !(200..300).contains(&status) {
            return Err(crate::status_error("Groq", status, &body, retry_after));
        }
        parse(&body)
    }
}

fn parse(body: &str) -> Result<Transcript, ChunkError> {
    let r: Response = serde_json::from_str(body)
        .map_err(|e| ChunkError::Fatal(format!("risposta Groq inattesa: {e}")))?;
    if !r.segments.is_empty() && r.segments.iter().all(|s| s.no_speech_prob > NO_SPEECH) {
        return Ok(Transcript::default());
    }
    Ok(Transcript {
        text: r.text.trim().to_owned(),
        language: r.language.as_deref().and_then(language_code),
        confidence: None,
    })
}

/// "italian" / "it" / "it-IT" → "it" (solo le lingue note all'app).
fn language_code(value: &str) -> Option<String> {
    let v = value.trim().to_ascii_lowercase();
    let code = match v.as_str() {
        "italian" => "it",
        "english" => "en",
        "spanish" => "es",
        "french" => "fr",
        "german" => "de",
        _ => {
            let base = sv_stt::base_code(&v);
            return (base.len() == 2).then_some(base);
        }
    };
    Some(code.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_and_language() {
        let t = parse(
            r#"{"task":"transcribe","language":"italian","duration":3.2,
            "text":" Ciao a tutti. ","segments":[{"id":0,"no_speech_prob":0.01}]}"#,
        )
        .unwrap();
        assert_eq!(t.text, "Ciao a tutti.");
        assert_eq!(t.language.as_deref(), Some("it"));
    }

    #[test]
    fn silence_hallucination_is_dropped() {
        let t = parse(
            r#"{"text":"Grazie per la visione.","segments":[{"no_speech_prob":0.9},{"no_speech_prob":0.8}]}"#,
        )
        .unwrap();
        assert!(t.text.is_empty());
        // Senza segmenti (response_format diverso) si tiene il testo.
        assert_eq!(parse(r#"{"text":"ok"}"#).unwrap().text, "ok");
    }

    #[test]
    fn language_codes() {
        assert_eq!(language_code("English").as_deref(), Some("en"));
        assert_eq!(language_code("de").as_deref(), Some("de"));
        assert_eq!(language_code("pt-BR").as_deref(), Some("pt"));
        assert_eq!(language_code("portuguese"), None);
    }
}
