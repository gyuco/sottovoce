//! Google Cloud Speech-to-Text v1, `speech:recognize` (sincrono, max ~1
//! minuto per richiesta) con API key. Lingua automatica: italiano più al
//! massimo 3 lingue alternative (limite dell'API: inglese, spagnolo,
//! francese; il tedesco resta fuori) e il servizio dichiara quella scelta.

use std::sync::Arc;

use base64::Engine as _;
use serde::Deserialize;
use serde_json::json;
use sv_stt::Language;

use crate::{ChunkError, Transcriber, Transcript};

const ENDPOINT: &str = "https://speech.googleapis.com/v1/speech:recognize";
pub const MODEL_FIXED: &str = "latest_long";
/// Le lingue alternative non sono garantite con `latest_long`.
pub const MODEL_MULTI: &str = "default";
const ALTERNATIVES: [&str; 3] = ["en-US", "es-ES", "fr-FR"];

#[derive(Debug, Clone)]
pub struct GoogleConfig {
    pub api_key: String,
}

pub struct Google {
    config: GoogleConfig,
    http: reqwest::Client,
}

impl Google {
    pub fn new(config: GoogleConfig) -> Result<Self, ChunkError> {
        if config.api_key.trim().is_empty() {
            return Err(ChunkError::Fatal("API key Google mancante".into()));
        }
        Ok(Self {
            config,
            http: crate::http_client(),
        })
    }
}

fn body(samples: &[f32], language: Language) -> serde_json::Value {
    let audio = base64::engine::general_purpose::STANDARD.encode(sv_stt::pcm16(samples));
    let mut config = json!({
        "encoding": "LINEAR16",
        "sampleRateHertz": 16000,
        "audioChannelCount": 1,
        "languageCode": crate::locale(language),
        "enableAutomaticPunctuation": true,
        "model": if language == Language::Multi { MODEL_MULTI } else { MODEL_FIXED },
    });
    if language == Language::Multi {
        config["alternativeLanguageCodes"] = json!(ALTERNATIVES);
    }
    json!({ "config": config, "audio": { "content": audio } })
}

#[derive(Debug, Default, Deserialize)]
struct Response {
    #[serde(default)]
    results: Vec<SpeechResult>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpeechResult {
    #[serde(default)]
    alternatives: Vec<Alternative>,
    #[serde(default)]
    language_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Alternative {
    #[serde(default)]
    transcript: String,
    #[serde(default)]
    confidence: Option<f32>,
}

impl Transcriber for Google {
    fn name(&self) -> &'static str {
        "google"
    }

    async fn transcribe(
        &self,
        samples: Arc<[f32]>,
        language: Language,
    ) -> Result<Transcript, ChunkError> {
        let req = self
            .http
            .post(ENDPOINT)
            .header("x-goog-api-key", self.config.api_key.trim())
            .json(&body(&samples, language));
        let (status, body, retry_after) = crate::send(req).await?;
        if !(200..300).contains(&status) {
            // Chiave sbagliata: Google risponde 400 con API_KEY_INVALID.
            if status == 400
                && (body.contains("API_KEY_INVALID") || body.contains("API key not valid"))
            {
                return Err(ChunkError::Unauthorized);
            }
            // 403 con l'API non abilitata nel progetto: il messaggio spiega cosa fare.
            if status == 403 && body.contains("SERVICE_DISABLED") {
                return Err(ChunkError::Fatal(format!(
                    "Google: API Speech-to-Text non abilitata nel progetto: {}",
                    crate::short(&body)
                )));
            }
            return Err(crate::status_error("Google", status, &body, retry_after));
        }
        parse(&body)
    }
}

fn parse(body: &str) -> Result<Transcript, ChunkError> {
    let r: Response = serde_json::from_str(body)
        .map_err(|e| ChunkError::Fatal(format!("risposta Google inattesa: {e}")))?;
    let mut text = String::new();
    let mut confidence: Option<f32> = None;
    let mut language = None;
    for result in &r.results {
        let Some(alt) = result.alternatives.first() else {
            continue;
        };
        let piece = alt.transcript.trim();
        if piece.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(piece);
        if let Some(c) = alt.confidence.filter(|c| *c > 0.0) {
            confidence = Some(confidence.map_or(c, |p| p.min(c)));
        }
        if language.is_none() {
            language = result
                .language_code
                .as_deref()
                .map(sv_stt::base_code)
                .filter(|l| !l.is_empty());
        }
    }
    Ok(Transcript {
        text,
        language,
        confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body() {
        let fixed = body(&[0.0; 4], Language::De);
        assert_eq!(fixed["config"]["languageCode"], "de-DE");
        assert_eq!(fixed["config"]["model"], MODEL_FIXED);
        assert!(fixed["config"]["alternativeLanguageCodes"].is_null());
        assert_eq!(fixed["audio"]["content"], "AAAAAAAAAAA=");
        let multi = body(&[0.0; 4], Language::Multi);
        assert_eq!(multi["config"]["languageCode"], "it-IT");
        assert_eq!(
            multi["config"]["alternativeLanguageCodes"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn joins_results_and_reads_language() {
        let t = parse(
            r#"{"results":[{"alternatives":[{"transcript":"we ship","confidence":0.9}],
            "resultEndTime":"1.2s","languageCode":"en-us"},
            {"alternatives":[{"transcript":" next week","confidence":0.8}]}],
            "totalBilledTime":"2s"}"#,
        )
        .unwrap();
        assert_eq!(t.text, "we ship next week");
        assert_eq!(t.language.as_deref(), Some("en"));
        assert_eq!(t.confidence, Some(0.8));
        // Nessun parlato: risposta senza `results`.
        assert_eq!(
            parse(r#"{"totalBilledTime":"1s"}"#).unwrap(),
            Transcript::default()
        );
    }
}
