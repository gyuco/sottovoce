//! Riepilogo finale del meeting in tre estrazioni parallele:
//! overview narrativa, key points per topic, action items.
//!
//! Un errore su una estrazione non blocca le altre: si salva ciò che è
//! arrivato. Il modello è quello del live assist (o della traduzione).

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use sv_llm::{ChatRequest, LlmProvider, Message, parse_json};
use sv_store::{ActionItem, KeyPoint, NewSummary};
use sv_stt::Language;

use crate::compress::{CompressLine, CompressionEvent, clock, format_block, language_name};
use crate::meeting::MeetingContext;

const TIMEOUT: Duration = Duration::from_secs(60);
const MAX_CONTEXT_CHARS: usize = 40_000;

pub struct SummaryConfig {
    pub llm: Arc<dyn LlmProvider>,
    pub model: String,
    /// Lingua in cui scrivere il riepilogo (concreta: `it` o `en`).
    pub language: Language,
}

pub struct SummaryInput {
    pub context: MeetingContext,
    pub blocks: Vec<CompressionEvent>,
    pub lines: Vec<CompressLine>,
}

/// Genera il riepilogo; `None` se tutte e tre le estrazioni falliscono.
pub async fn generate(cfg: &SummaryConfig, input: &SummaryInput) -> Option<NewSummary> {
    let material = material(input);
    let (overview, key_points, checklist) = tokio::join!(
        overview(cfg, &material),
        key_points(cfg, &material),
        action_items(cfg, &material),
    );
    if overview.is_none() && key_points.is_none() && checklist.is_none() {
        tracing::warn!("riepilogo finale: tutte le estrazioni fallite");
        return None;
    }
    Some(NewSummary {
        language: cfg.language.code().to_owned(),
        overview: overview.unwrap_or_default(),
        key_points: key_points.unwrap_or_default(),
        action_items: checklist
            .unwrap_or_default()
            .into_iter()
            .map(|text| ActionItem { text, done: false })
            .collect(),
    })
}

fn material(input: &SummaryInput) -> String {
    let mut out = String::new();
    if let Some(block) = input.context.prompt_block() {
        out.push_str(&block);
        out.push('\n');
    }
    if !input.blocks.is_empty() {
        out.push_str("\nCOMPRESSED MEETING SO FAR:\n");
        for block in &input.blocks {
            out.push_str(&format_block(block));
            out.push('\n');
        }
    }
    if !input.lines.is_empty() {
        out.push_str("\nLAST TRANSCRIPT:\n");
        for line in &input.lines {
            out.push('[');
            out.push_str(&clock(line.start_ms));
            out.push_str("] ");
            match line.channel {
                sv_audio::Channel::Me => out.push_str("[Io]"),
                sv_audio::Channel::Them => {
                    out.push('[');
                    out.push_str(&input.context.speaker_label(line.speaker));
                    out.push(']');
                }
            }
            out.push(' ');
            out.push_str(line.text.trim());
            out.push('\n');
        }
    }
    if out.len() > MAX_CONTEXT_CHARS {
        // Meglio la coda (le frasi più recenti) che l'inizio.
        let mut start = out.len() - MAX_CONTEXT_CHARS;
        while !out.is_char_boundary(start) {
            start += 1;
        }
        out = format!("…\n{}", &out[start..]);
    }
    out
}

async fn overview(cfg: &SummaryConfig, material: &str) -> Option<String> {
    let system = format!(
        "You are an expert meeting summarizer. Given a meeting transcript and its \
         compressed history, produce a short narrative summary.\n\
         Single paragraph, no lists. Who was there, what it was about, main threads. \
         Mention participants by name, high-level, no verbatim quotes, no opinions. \
         3-5 sentences, max 120 words, past tense, third person. \
         Write in {}. Respond only with the paragraph.",
        language_name(cfg.language)
    );
    let text = ask(cfg, system, material, false).await.ok()?;
    let text = clean(&text);
    (!text.is_empty()).then_some(text)
}

#[derive(Deserialize)]
struct KeyPointsOutput {
    #[serde(default)]
    key_points: Vec<KeyPoint>,
}

async fn key_points(cfg: &SummaryConfig, material: &str) -> Option<Vec<KeyPoint>> {
    let system = format!(
        "Identify 2-5 major topics from the meeting; under each, 2-5 points attributed to \
         who raised them (\"Name said/raised/confirmed...\"). One concrete sentence each, \
         factual, past tense. Weave any checklist/goal from the meeting context into topics.\n\
         Write in {}.\n\
         JSON only: {{\"key_points\": [{{\"topic\": \"...\", \"points\": [\"...\"]}}]}}",
        language_name(cfg.language)
    );
    let raw = ask(cfg, system, material, true).await.ok()?;
    let out: KeyPointsOutput = parse_json(&raw).ok()?;
    let points: Vec<KeyPoint> = out
        .key_points
        .into_iter()
        .filter_map(|item| {
            let topic = item.topic.trim().to_owned();
            let points = topic_points(item.points);
            (!topic.is_empty() && !points.is_empty()).then_some(KeyPoint { topic, points })
        })
        .take(5)
        .collect();
    (!points.is_empty()).then_some(points)
}

#[derive(Deserialize)]
struct ChecklistOutput {
    #[serde(default)]
    checklist: Vec<String>,
}

async fn action_items(cfg: &SummaryConfig, material: &str) -> Option<Vec<String>> {
    let system = format!(
        "Extract action items, follow-ups, open questions and tasks to handle AFTER the \
         meeting. Include the responsible person and deadlines if mentioned.\n\
         Good: \"John to send the proposal by Friday\". Bad: \"Discussed the project\".\n\
         Order by priority if discernible. 3-10 items, never invent; empty array if none.\n\
         Write in {}.\n\
         JSON only: {{\"checklist\": [\"...\"]}}",
        language_name(cfg.language)
    );
    let raw = ask(cfg, system, material, true).await.ok()?;
    let out: ChecklistOutput = parse_json(&raw).ok()?;
    let mut seen = std::collections::HashSet::new();
    let items: Vec<String> = out
        .checklist
        .into_iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty() && seen.insert(item.to_lowercase()))
        .take(10)
        .collect();
    (!items.is_empty()).then_some(items)
}

fn topic_points(points: Vec<String>) -> Vec<String> {
    points
        .into_iter()
        .map(|point| point.trim().to_owned())
        .filter(|point| !point.is_empty())
        .take(5)
        .collect()
}

async fn ask(
    cfg: &SummaryConfig,
    system: String,
    material: &str,
    json: bool,
) -> Result<String, sv_llm::LlmError> {
    let mut req = ChatRequest::new(
        cfg.model.clone(),
        vec![Message::system(system), Message::user(material)],
    )
    .temperature(0.2)
    .timeout(TIMEOUT);
    if json {
        req = req.json();
    }
    cfg.llm.chat(req).await.map(|r| r.content)
}

/// Toglie il ragionamento ` think…</think>` e le virgolette di alcuni modelli.
fn clean(content: &str) -> String {
    let mut text = content;
    if let Some(end) = text.find("</think>") {
        text = &text[end + "</think>".len()..];
    }
    let text = text.trim();
    let pairs = [('"', '"'), ('“', '”'), ('«', '»')];
    for (open, close) in pairs {
        if let Some(inner) = text.strip_prefix(open).and_then(|t| t.strip_suffix(close)) {
            return inner.trim().to_owned();
        }
    }
    text.to_owned()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use sv_llm::{BoxFuture, ChatResponse, LlmError, ModelInfo};

    use super::*;

    /// Risponde in base al prompt: overview, key points, checklist.
    struct RoutingLlm {
        fail_overview: bool,
        requests: Mutex<Vec<Vec<Message>>>,
    }

    impl LlmProvider for RoutingLlm {
        fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<ChatResponse, LlmError>> {
            let system = req.messages[0].content.clone();
            self.requests.lock().unwrap().push(req.messages);
            let fail = self.fail_overview && system.contains("narrative summary");
            Box::pin(async move {
                if fail {
                    return Err(LlmError::Timeout);
                }
                let content = if system.contains("narrative summary") {
                    "Il team ha discusso il budget e la data della beta.".to_owned()
                } else if system.contains("major topics") {
                    r#"{"key_points":[{"topic":"Budget","points":["Marco ha confermato il budget."]},{"topic":"","points":["x"]}]}"#.to_owned()
                } else {
                    r#"{"checklist":["Marco invia il preventivo entro venerdì","Marco invia il preventivo entro venerdì",""]}"#.to_owned()
                };
                Ok(ChatResponse {
                    content,
                    model: None,
                    usage: None,
                    latency: Duration::from_millis(5),
                    attempts: 1,
                })
            })
        }

        fn list_models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>, LlmError>> {
            Box::pin(async { Ok(vec![]) })
        }
    }

    fn input() -> SummaryInput {
        SummaryInput {
            context: MeetingContext {
                name: "Beta".into(),
                ..MeetingContext::default()
            },
            blocks: Vec::new(),
            lines: vec![CompressLine {
                channel: sv_audio::Channel::Me,
                text: "Chiudiamo venerdì".into(),
                speaker: None,
                start_ms: 0,
                end_ms: 1_000,
            }],
        }
    }

    fn config(llm: Arc<RoutingLlm>) -> SummaryConfig {
        SummaryConfig {
            llm,
            model: "m".into(),
            language: Language::It,
        }
    }

    #[tokio::test]
    async fn parses_the_three_extractions() {
        let llm = Arc::new(RoutingLlm {
            fail_overview: false,
            requests: Mutex::default(),
        });
        let summary = generate(&config(llm), &input()).await.unwrap();
        assert_eq!(summary.language, "it");
        assert!(summary.overview.contains("budget"));
        assert_eq!(summary.key_points.len(), 1);
        assert_eq!(summary.key_points[0].topic, "Budget");
        assert_eq!(summary.action_items.len(), 1);
        assert!(!summary.action_items[0].done);
        assert_eq!(
            summary.action_items[0].text,
            "Marco invia il preventivo entro venerdì"
        );
    }

    #[tokio::test]
    async fn one_failed_extraction_does_not_block_the_others() {
        let llm = Arc::new(RoutingLlm {
            fail_overview: true,
            requests: Mutex::default(),
        });
        let summary = generate(&config(llm), &input()).await.unwrap();
        assert!(summary.overview.is_empty());
        assert_eq!(summary.key_points.len(), 1);
        assert_eq!(summary.action_items.len(), 1);
    }

    #[test]
    fn cleans_model_output() {
        assert_eq!(clean(" thinkx</think>\nCiao"), "Ciao");
        assert_eq!(clean("«Ciao»"), "Ciao");
    }
}
