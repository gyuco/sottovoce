//! Compressione rolling della conversazione ogni [`PERIOD`].
//!
//! Un task riceve le frasi finali da una coda, le accumula e ogni 5 minuti
//! chiede al modello un riassunto JSON (`summary`, `topics`,
//! `important_moments`). I blocchi restano in memoria (per live assist e
//! riepilogo finale) e vengono salvati su SQLite. Se la call finisce prima
//! del tick, il blocco parziale non viene compresso: il riepilogo finale usa
//! comunque la coda recente della trascrizione.

use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sv_audio::Channel;
use sv_llm::{ChatRequest, LlmProvider, Message, parse_json};
use sv_store::{ImportantMoment, NewCompressionBlock, StoreHandle};
use sv_stt::Language;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::meeting::SharedContext;

/// Ogni quanto comprimere.
pub const PERIOD: Duration = Duration::from_secs(5 * 60);
/// Frasi in attesa fra due blocchi.
pub const QUEUE: usize = 512;
/// Frasi recenti conservate per il riepilogo finale.
pub const RECENT_LINES: usize = 80;
/// Budget indicativo del contesto compresso nell'assist (~8000 token).
pub const MAX_CONTEXT_CHARS: usize = 28_000;
const TIMEOUT: Duration = Duration::from_secs(60);

/// Frase finale che entra nella compressione.
#[derive(Debug, Clone, PartialEq)]
pub struct CompressLine {
    pub channel: Channel,
    pub text: String,
    pub speaker: Option<u8>,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Blocco pronto per la UI e per la memoria della sessione.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressionEvent {
    pub index: u32,
    pub start: u64,
    pub end: u64,
    pub summary: String,
    pub topics: Vec<String>,
    pub moments: Vec<ImportantMoment>,
}

pub struct CompressConfig {
    pub llm: Arc<dyn LlmProvider>,
    pub model: String,
    /// Lingua in cui scrivere riassunti e momenti.
    pub language: Language,
}

/// Blocchi e frasi recenti condivisi da compressione, assist e riepilogo.
#[derive(Clone, Default)]
pub struct CompressionLog(Arc<RwLock<Log>>);

#[derive(Default)]
struct Log {
    blocks: Vec<CompressionEvent>,
    recent: VecDeque<CompressLine>,
}

impl CompressionLog {
    pub fn push_line(&self, line: CompressLine) {
        let mut log = self.0.write().unwrap_or_else(|e| e.into_inner());
        if log.recent.len() == RECENT_LINES {
            log.recent.pop_front();
        }
        log.recent.push_back(line);
    }

    pub fn add_block(&self, block: CompressionEvent) {
        self.0
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .blocks
            .push(block);
    }

    pub fn blocks(&self) -> Vec<CompressionEvent> {
        self.0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .blocks
            .clone()
    }

    pub fn lines(&self) -> Vec<CompressLine> {
        self.0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .recent
            .iter()
            .cloned()
            .collect()
    }

    /// Blocchi compressi entro `max_chars`, dal più recente all'indietro, poi
    /// rimessi in ordine cronologico. `None` se non c'è ancora nessun blocco.
    pub fn prompt_block(&self, max_chars: usize) -> Option<String> {
        let blocks = self.blocks();
        let mut pieces: Vec<String> = Vec::new();
        let mut used = 0usize;
        for b in blocks.iter().rev() {
            let piece = format_block(b);
            if used + piece.len() > max_chars {
                if pieces.is_empty() {
                    pieces.push(piece.chars().take(max_chars).collect());
                }
                break;
            }
            used += piece.len() + 2;
            pieces.push(piece);
        }
        if pieces.is_empty() {
            return None;
        }
        pieces.reverse();
        Some(pieces.join("\n\n"))
    }
}

pub(crate) fn format_block(b: &CompressionEvent) -> String {
    let mut out = format!("[{}–{}] {}", clock(b.start), clock(b.end), b.summary.trim());
    if !b.topics.is_empty() {
        out.push_str("\nTopics: ");
        out.push_str(&b.topics.join(", "));
    }
    for m in &b.moments {
        out.push_str("\n- ");
        out.push_str(&clock(m.time_ms));
        out.push(' ');
        out.push_str(&m.kind);
        out.push_str(": ");
        out.push_str(m.text.trim());
    }
    out
}

/// `mm:ss` (o `h:mm:ss`).
pub fn clock(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Avvia il task. Chiudere il `Sender` (drop) lo ferma.
pub fn spawn(
    cfg: CompressConfig,
    context: SharedContext,
    log: CompressionLog,
    store: Option<StoreHandle>,
    on_block: impl Fn(CompressionEvent) + Send + Sync + 'static,
) -> (mpsc::Sender<CompressLine>, JoinHandle<()>) {
    spawn_with_period(cfg, context, log, store, on_block, PERIOD)
}

pub(crate) fn spawn_with_period(
    cfg: CompressConfig,
    context: SharedContext,
    log: CompressionLog,
    store: Option<StoreHandle>,
    on_block: impl Fn(CompressionEvent) + Send + Sync + 'static,
    period: Duration,
) -> (mpsc::Sender<CompressLine>, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(QUEUE);
    let task = tokio::spawn(run(
        cfg,
        context,
        log,
        store,
        period,
        rx,
        Arc::new(on_block),
    ));
    (tx, task)
}

async fn run(
    cfg: CompressConfig,
    context: SharedContext,
    log: CompressionLog,
    store: Option<StoreHandle>,
    period: Duration,
    mut rx: mpsc::Receiver<CompressLine>,
    emit: Arc<dyn Fn(CompressionEvent) + Send + Sync>,
) {
    let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut pending: Vec<CompressLine> = Vec::new();
    let mut next_index = 1u32;
    let mut stats = Stats::default();

    loop {
        tokio::select! {
            line = rx.recv() => match line {
                Some(line) => {
                    if !line.text.trim().is_empty() {
                        log.push_line(line.clone());
                        pending.push(line);
                    }
                }
                None => break,
            },
            _ = ticks.tick() => {
                if pending.is_empty() {
                    stats.empty += 1;
                    continue;
                }
                let lines = std::mem::take(&mut pending);
                let start = lines.iter().map(|l| l.start_ms).min().unwrap_or(0);
                let end = lines.iter().map(|l| l.end_ms).max().unwrap_or(start);
                match summarize(&cfg, &context, &lines).await {
                    Some(output) => {
                        let block = CompressionEvent {
                            index: next_index,
                            start,
                            end,
                            summary: output.summary,
                            topics: output.topics,
                            moments: output.moments,
                        };
                        next_index += 1;
                        log.add_block(block.clone());
                        if let Some(store) = &store {
                            store.compression_block(NewCompressionBlock {
                                index: block.index,
                                start_ms: block.start,
                                end_ms: block.end,
                                summary: block.summary.clone(),
                                topics: block.topics.clone(),
                                moments: block.moments.clone(),
                            });
                        }
                        tracing::info!(block = block.index, start, end, "blocco di compressione salvato");
                        stats.blocks += 1;
                        emit(block);
                    }
                    None => stats.failed += 1,
                }
            }
        }
    }
    stats.report();
}

#[derive(Debug, Default, Deserialize)]
struct SegmentOutput {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    important_moments: Vec<MomentOutput>,
}

#[derive(Debug, Deserialize)]
struct MomentOutput {
    #[serde(default)]
    time: Option<serde_json::Value>,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

async fn summarize(
    cfg: &CompressConfig,
    context: &SharedContext,
    lines: &[CompressLine],
) -> Option<CompressionEventFields> {
    let start = lines.iter().map(|l| l.start_ms).min().unwrap_or(0);
    let end = lines.iter().map(|l| l.end_ms).max().unwrap_or(start);
    let req = ChatRequest::new(cfg.model.clone(), messages(cfg, context, start, end, lines))
        .json()
        .temperature(0.2)
        .timeout(TIMEOUT);
    let resp = match cfg.llm.chat(req).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("compressione 5 min: richiesta fallita: {e}");
            return None;
        }
    };
    let output: SegmentOutput = match parse_json(&resp.content) {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("compressione 5 min: JSON non valido, blocco saltato: {e}");
            return None;
        }
    };
    let summary = output.summary.trim().to_owned();
    if summary.is_empty() {
        tracing::warn!("compressione 5 min: riassunto vuoto, blocco saltato");
        return None;
    }
    let topics = output
        .topics
        .into_iter()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect();
    let moments = output
        .important_moments
        .into_iter()
        .filter_map(|m| {
            let text = m.text.trim();
            if text.is_empty() {
                return None;
            }
            let kind = match m.kind.trim() {
                "objection" | "commitment" | "question" | "pain_point" | "decision" => {
                    m.kind.trim().to_owned()
                }
                _ => "decision".to_owned(),
            };
            Some(ImportantMoment {
                time_ms: moment_time(m.time.as_ref(), start, end),
                kind,
                text: text.to_owned(),
            })
        })
        .collect();
    Some(CompressionEventFields {
        summary,
        topics,
        moments,
    })
}

struct CompressionEventFields {
    summary: String,
    topics: Vec<String>,
    moments: Vec<ImportantMoment>,
}

/// Il modello può restituire secondi o millisecondi: si riconosce dal range.
fn moment_time(value: Option<&serde_json::Value>, start: u64, end: u64) -> u64 {
    let seconds_or_ms = match value {
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(serde_json::Value::String(s)) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => return start,
    };
    let limit = end.saturating_add(60_000);
    let ms = if seconds_or_ms > 0.0 && seconds_or_ms <= (limit as f64 / 1000.0) {
        seconds_or_ms * 1000.0
    } else {
        seconds_or_ms
    };
    (ms.max(0.0) as u64).clamp(start, end.max(start))
}

fn messages(
    cfg: &CompressConfig,
    context: &SharedContext,
    start: u64,
    end: u64,
    lines: &[CompressLine],
) -> Vec<Message> {
    let lang = language_name(cfg.language);
    let mut system = format!(
        "You summarize a segment of an ongoing meeting for later reference.\n\
         Write in {lang}. Keep names, numbers, decisions and commitments exact.\n\
         Respond with JSON only:\n\
         {{\"summary\": \"3-4 concise bullet points separated by newlines\", \
           \"topics\": [\"...\"], \
           \"important_moments\": [{{\"time\": 305000, \"type\": \"objection|commitment|question|pain_point|decision\", \"text\": \"...\"}}]}}\n\
         \"time\" is milliseconds from the start of the meeting. Only include genuinely important moments."
    );
    if let Some(block) = context.prompt_block() {
        system.push_str("\n\n");
        system.push_str(&block);
    }
    let mut user = format!("Meeting segment {}–{}:\n", clock(start), clock(end));
    for line in lines {
        user.push('[');
        user.push_str(&clock(line.start_ms));
        user.push_str("] ");
        match line.channel {
            Channel::Me => user.push_str("[Io]"),
            Channel::Them => {
                user.push('[');
                user.push_str(&context.speaker_label(line.speaker));
                user.push(']');
            }
        }
        user.push(' ');
        user.push_str(line.text.trim());
        user.push('\n');
    }
    vec![Message::system(system), Message::user(user.trim_end())]
}

pub(crate) fn language_name(language: Language) -> &'static str {
    match language {
        Language::En => "English",
        Language::It | Language::Multi => "Italian",
    }
}

#[derive(Default)]
struct Stats {
    blocks: usize,
    failed: usize,
    empty: usize,
}

impl Stats {
    fn report(self) {
        tracing::info!(
            blocks = self.blocks,
            failed = self.failed,
            empty_ticks = self.empty,
            "riepilogo compressione"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use sv_llm::{BoxFuture, ChatResponse, LlmError, ModelInfo};

    use super::*;
    use crate::meeting::MeetingContext;

    struct ScriptedLlm {
        replies: Mutex<VecDeque<String>>,
        requests: Mutex<Vec<Vec<Message>>>,
    }

    impl ScriptedLlm {
        fn new(replies: &[&str]) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies.iter().map(|r| r.to_string()).collect()),
                requests: Mutex::default(),
            })
        }
    }

    impl LlmProvider for ScriptedLlm {
        fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<ChatResponse, LlmError>> {
            self.requests.lock().unwrap().push(req.messages);
            let reply = self.replies.lock().unwrap().pop_front().unwrap_or_default();
            Box::pin(async move {
                Ok(ChatResponse {
                    content: reply,
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

    fn line(id: u64) -> CompressLine {
        CompressLine {
            channel: if id.is_multiple_of(2) {
                Channel::Me
            } else {
                Channel::Them
            },
            text: format!("frase {id}"),
            speaker: Some(0),
            start_ms: id * 10_000,
            end_ms: id * 10_000 + 9_000,
        }
    }

    fn config(llm: Arc<ScriptedLlm>) -> CompressConfig {
        CompressConfig {
            llm,
            model: "m".into(),
            language: Language::It,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn compresses_only_when_there_are_new_lines() {
        let llm = ScriptedLlm::new(&[
            r#"{"summary":"- punto uno","topics":["budget"],"important_moments":[{"time":10,"type":"decision","text":"si chiude venerdì"}]}"#,
        ]);
        let blocks = Arc::new(Mutex::new(Vec::new()));
        let sink = blocks.clone();
        let log = CompressionLog::default();
        let (tx, task) = spawn_with_period(
            config(llm.clone()),
            SharedContext::new(MeetingContext::default()),
            log.clone(),
            None,
            move |b| sink.lock().unwrap().push(b),
            Duration::from_secs(60),
        );
        // Tick a vuoto: nessuna chiamata.
        tokio::time::sleep(Duration::from_secs(61)).await;
        assert_eq!(llm.requests.lock().unwrap().len(), 0);

        tx.send(line(1)).await.unwrap();
        tokio::time::sleep(Duration::from_secs(61)).await;
        drop(tx);
        task.await.unwrap();

        let blocks = blocks.lock().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].index, 1);
        assert_eq!(blocks[0].summary, "- punto uno");
        assert_eq!(blocks[0].topics, ["budget"]);
        assert_eq!(blocks[0].moments[0].time_ms, 10_000);
        assert_eq!(log.blocks().len(), 1);
        assert_eq!(log.lines().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn invalid_json_skips_the_block() {
        let llm = ScriptedLlm::new(&["non è JSON"]);
        let log = CompressionLog::default();
        let (tx, task) = spawn_with_period(
            config(llm),
            SharedContext::new(MeetingContext::default()),
            log.clone(),
            None,
            |_| {},
            Duration::from_secs(60),
        );
        tx.send(line(1)).await.unwrap();
        tokio::time::sleep(Duration::from_secs(61)).await;
        drop(tx);
        task.await.unwrap();
        assert!(log.blocks().is_empty());
    }

    #[test]
    fn prompt_block_fits_the_budget_and_keeps_the_latest() {
        let log = CompressionLog::default();
        for i in 1..=20u32 {
            log.add_block(CompressionEvent {
                index: i,
                start: u64::from(i) * 300_000,
                end: u64::from(i) * 300_000 + 299_000,
                summary: format!("blocco numero {i} {}", "x".repeat(100)),
                topics: vec![format!("topic {i}")],
                moments: Vec::new(),
            });
        }
        let block = log.prompt_block(2_000).unwrap();
        assert!(block.len() <= 2_000);
        assert!(block.contains("blocco numero 20"));
        assert!(!block.contains("blocco numero 1 "));
    }
}
