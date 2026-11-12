//! Azure AI Speech, REST "short audio" (max 60 s per richiesta), che rientra
//! nelle ore gratuite di trascrizione in tempo reale del piano F0. Una lingua
//! per richiesta: in modalità automatica si usa la lingua di riserva
//! (quella dell'utente) e la lingua della frase si stima dal testo.

use std::sync::Arc;

use serde::Deserialize;
use sv_stt::Language;

use crate::{ChunkError, Transcriber, Transcript};

#[derive(Debug, Clone)]
pub struct AzureConfig {
    pub api_key: String,
    /// Regione della risorsa Speech (es. `westeurope`).
    pub region: String,
    /// Lingua usata quando la call è in modalità automatica.
    pub fallback: Language,
}

pub struct Azure {
    config: AzureConfig,
    http: reqwest::Client,
}

impl Azure {
    pub fn new(config: AzureConfig) -> Result<Self, ChunkError> {
        if config.api_key.trim().is_empty() {
            return Err(ChunkError::Fatal("API key Azure mancante".into()));
        }
        if !valid_region(&config.region) {
            return Err(ChunkError::Fatal(format!(
                "regione Azure non valida: \"{}\" (es. westeurope)",
                config.region.trim()
            )));
        }
        Ok(Self {
            config,
            http: crate::http_client(),
        })
    }

    fn url(&self, language: Language) -> String {
        let language = match language {
            Language::Multi => self.config.fallback,
            fixed => fixed,
        };
        format!(
            "https://{}.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language={}&format=detailed",
            self.config.region.trim().to_ascii_lowercase(),
            crate::locale(language)
        )
    }
}

/// Solo lettere e cifre: la regione finisce nel nome host.
fn valid_region(region: &str) -> bool {
    let r = region.trim();
    !r.is_empty() && r.len() <= 40 && r.chars().all(|c| c.is_ascii_alphanumeric())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Response {
    recognition_status: String,
    #[serde(default)]
    display_text: Option<String>,
    #[serde(default, rename = "NBest")]
    n_best: Vec<NBest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct NBest {
    #[serde(default)]
    confidence: Option<f32>,
    #[serde(default)]
    display: Option<String>,
}

impl Transcriber for Azure {
    fn name(&self) -> &'static str {
        "azure"
    }

    async fn transcribe(
        &self,
        samples: Arc<[f32]>,
        language: Language,
    ) -> Result<Transcript, ChunkError> {
        let req = self
            .http
            .post(self.url(language))
            .header("Ocp-Apim-Subscription-Key", self.config.api_key.trim())
            .header(
                "Content-Type",
                "audio/wav; codecs=audio/pcm; samplerate=16000",
            )
            .header("Accept", "application/json")
            .body(crate::wav(&samples));
        let (status, body, retry_after) = crate::send(req).await?;
        if !(200..300).contains(&status) {
            return Err(crate::status_error("Azure", status, &body, retry_after));
        }
        parse(&body)
    }
}

fn parse(body: &str) -> Result<Transcript, ChunkError> {
    let r: Response = serde_json::from_str(body)
        .map_err(|e| ChunkError::Fatal(format!("risposta Azure inattesa: {e}")))?;
    match r.recognition_status.as_str() {
        "Success" => {
            let best = r.n_best.first();
            let text = r
                .display_text
                .or_else(|| best.and_then(|b| b.display.clone()))
                .unwrap_or_default();
            Ok(Transcript {
                text: text.trim().to_owned(),
                language: None,
                confidence: best.and_then(|b| b.confidence),
            })
        }
        // Nessun parlato riconosciuto.
        "NoMatch" | "InitialSilenceTimeout" | "BabbleTimeout" => Ok(Transcript::default()),
        other => Err(ChunkError::Temporary(format!("Azure: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn azure(region: &str) -> Result<Azure, ChunkError> {
        Azure::new(AzureConfig {
            api_key: "k".into(),
            region: region.into(),
            fallback: Language::Es,
        })
    }

    #[test]
    fn url_and_region() {
        let a = azure(" WestEurope ").unwrap();
        assert_eq!(
            a.url(Language::It),
            "https://westeurope.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language=it-IT&format=detailed"
        );
        assert!(a.url(Language::Multi).contains("language=es-ES"));
        for bad in ["", "evil.com/x", "west europe", "a@b"] {
            assert!(azure(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn parses_statuses() {
        let t = parse(
            r#"{"RecognitionStatus":"Success","Offset":1000,"Duration":2000,
            "DisplayText":"Ciao a tutti.","NBest":[{"Confidence":0.93,"Display":"Ciao a tutti."}]}"#,
        )
        .unwrap();
        assert_eq!(t.text, "Ciao a tutti.");
        assert_eq!(t.confidence, Some(0.93));
        assert_eq!(
            parse(r#"{"RecognitionStatus":"NoMatch","Offset":0,"Duration":0}"#).unwrap(),
            Transcript::default()
        );
        assert!(parse(r#"{"RecognitionStatus":"Error"}"#).is_err());
    }
}
