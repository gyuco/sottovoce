//! Traduzione per frase dei `Final` del canale "Loro".
//!
//! Un task riceve le frasi da una coda limitata e le traduce con al massimo
//! [`MAX_PARALLEL`] richieste LLM in parallelo. Le traduzioni possono
//! arrivare fuori ordine: la UI le associa alla frase tramite `segment_id`.
//! Un errore (LLM spento, timeout, ...) lascia la frase senza traduzione e
//! non ferma né la coda né la trascrizione.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use sv_llm::{ChatRequest, LlmProvider, Message};
use sv_stt::Language;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::{JoinHandle, JoinSet};

use crate::meeting::SharedContext;
use crate::session::percentiles;

/// Richieste LLM contemporanee.
const MAX_PARALLEL: usize = 2;
/// Frasi in attesa oltre le quali si scarta (l'LLM non tiene il passo).
pub const QUEUE: usize = 16;
/// Timeout per frase: oltre, la traduzione arriverebbe comunque troppo tardi.
const TIMEOUT: Duration = Duration::from_secs(8);
/// Frasi precedenti passate come contesto.
const HISTORY: usize = 3;

/// Cosa serve per tradurre, preparato dal comando `start_session`.
#[derive(Clone)]
pub struct TranslatorConfig {
    pub llm: Arc<dyn LlmProvider>,
    pub model: String,
    /// Lingua di arrivo (concreta: mai `Multi`).
    pub target: Language,
    /// Lingua impostata per "Loro", usata se il motore non ne rileva una.
    pub source_hint: Option<Language>,
}

/// Frase da tradurre.
#[derive(Debug, Clone)]
pub struct Job {
    pub segment_id: u64,
    pub text: String,
    /// Lingua rilevata dal motore STT, se c'è.
    pub language: Option<String>,
    /// Parlante, se riconosciuto (etichetta nel contesto delle frasi).
    pub speaker: Option<u8>,
}

/// Traduzione pronta, inviata alla UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationEvent {
    pub segment_id: u64,
    pub text: String,
    pub latency_ms: u64,
}

/// Avvia il traduttore. Chiudere il `Sender` (drop) lo ferma dopo aver
/// completato le richieste in corso.
pub fn spawn(
    cfg: TranslatorConfig,
    context: SharedContext,
    on_translation: impl Fn(TranslationEvent) + Send + Sync + 'static,
) -> (mpsc::Sender<Job>, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(QUEUE);
    let task = tokio::spawn(run(cfg, context, rx, Arc::new(on_translation)));
    (tx, task)
}

enum Outcome {
    Done(u64),
    Failed,
}

async fn run(
    cfg: TranslatorConfig,
    context: SharedContext,
    mut rx: mpsc::Receiver<Job>,
    emit: Arc<dyn Fn(TranslationEvent) + Send + Sync>,
) {
    let cfg = Arc::new(cfg);
    let slots = Arc::new(Semaphore::new(MAX_PARALLEL));
    let mut inflight = JoinSet::new();
    let mut history: VecDeque<String> = VecDeque::with_capacity(HISTORY);
    let mut stats = Stats::default();

    while let Some(job) = rx.recv().await {
        while let Some(done) = inflight.try_join_next() {
            stats.record(done.ok());
        }
        let language = job
            .language
            .clone()
            .or_else(|| cfg.source_hint.map(|l| l.code().to_owned()));
        let skip = same_language(language.as_deref(), cfg.target) || !has_letters(&job.text);
        let previous: Vec<String> = history.iter().cloned().collect();
        if history.len() == HISTORY {
            history.pop_front();
        }
        history.push_back(match job.speaker {
            Some(_) => format!("{}: {}", context.speaker_label(job.speaker), job.text),
            None => job.text.clone(),
        });
        if skip {
            stats.skipped += 1;
            continue;
        }

        // Attende uno slot: intanto le frasi successive restano in coda.
        let Ok(permit) = slots.clone().acquire_owned().await else {
            break;
        };
        let (cfg, emit) = (cfg.clone(), emit.clone());
        let block = context.prompt_block();
        inflight.spawn(async move {
            let _permit = permit;
            translate(&cfg, block, &previous, job, emit.as_ref()).await
        });
    }

    while let Some(done) = inflight.join_next().await {
        stats.record(done.ok());
    }
    stats.report();
}

async fn translate(
    cfg: &TranslatorConfig,
    context: Option<String>,
    previous: &[String],
    job: Job,
    emit: &(dyn Fn(TranslationEvent) + Send + Sync),
) -> Outcome {
    match translate_one(cfg, context.as_deref(), previous, &job.text).await {
        Ok((text, latency_ms)) => {
            emit(TranslationEvent {
                segment_id: job.segment_id,
                text,
                latency_ms,
            });
            Outcome::Done(latency_ms)
        }
        Err(e) => {
            tracing::warn!(segment_id = job.segment_id, "traduzione fallita: {e}");
            Outcome::Failed
        }
    }
}

/// Traduce una frase: (testo, latenza in ms). Usata dal traduttore
/// automatico e dal pulsante "Traduci".
pub async fn translate_one(
    cfg: &TranslatorConfig,
    context: Option<&str>,
    previous: &[String],
    text: &str,
) -> Result<(String, u64), String> {
    let req = ChatRequest::new(
        cfg.model.clone(),
        messages(cfg.target, context, previous, text),
    )
    .temperature(0.2)
    .timeout(TIMEOUT);
    let resp = cfg.llm.chat(req).await.map_err(|e| e.to_string())?;
    let text = clean(&resp.content);
    if text.is_empty() {
        return Err("traduzione vuota".into());
    }
    let latency_ms = u64::try_from(resp.latency.as_millis()).unwrap_or(u64::MAX);
    Ok((text, latency_ms))
}

/// Prompt di traduzione: system con istruzioni, contesto meeting e frasi
/// precedenti; user con la sola frase da tradurre.
pub fn messages(
    target: Language,
    context: Option<&str>,
    previous: &[String],
    text: &str,
) -> Vec<Message> {
    let lang = language_name(target);
    let mut system = format!(
        "You are a real-time meeting interpreter. Translate the user's message into {lang}.\n\
         Output only the translation: no quotes, notes, or explanations.\n\
         Be faithful and concise; keep names, numbers, acronyms, and technical terms.\n\
         The text comes from speech recognition and may contain errors: translate the most likely meaning.\n\
         If the message is already in {lang}, output it unchanged."
    );
    if let Some(block) = context {
        system.push_str("\n\n");
        system.push_str(block);
    }
    if !previous.is_empty() {
        system.push_str("\n\nPrevious sentences (context only, do not translate):");
        for p in previous {
            system.push_str("\n- ");
            system.push_str(p);
        }
    }
    vec![Message::system(system), Message::user(text)]
}

fn language_name(l: Language) -> &'static str {
    match l {
        Language::En => "English",
        Language::It | Language::Multi => "Italian",
    }
}

/// `language` (es. `en`, `en-US`) coincide con la lingua di arrivo.
fn same_language(language: Option<&str>, target: Language) -> bool {
    language
        .and_then(|l| l.split(['-', '_']).next())
        .is_some_and(|l| l.eq_ignore_ascii_case(target.code()))
}

fn has_letters(text: &str) -> bool {
    text.chars().any(char::is_alphabetic)
}

/// Toglie il ragionamento `<think>…</think>` di alcuni modelli locali e le
/// virgolette che a volte avvolgono la risposta.
fn clean(content: &str) -> String {
    let mut text = content;
    if let Some(end) = text.find("</think>") {
        text = &text[end + "</think>".len()..];
    }
    let text = text.trim();
    let pairs = [('"', '"'), ('“', '”'), ('«', '»'), ('\'', '\'')];
    for (open, close) in pairs {
        if let Some(inner) = text.strip_prefix(open).and_then(|t| t.strip_suffix(close)) {
            return inner.trim().to_owned();
        }
    }
    text.to_owned()
}

#[derive(Default)]
struct Stats {
    latencies: Vec<u64>,
    failed: usize,
    skipped: usize,
}

impl Stats {
    fn record(&mut self, outcome: Option<Outcome>) {
        match outcome {
            Some(Outcome::Done(ms)) => self.latencies.push(ms),
            Some(Outcome::Failed) | None => self.failed += 1,
        }
    }

    fn report(mut self) {
        let [p50, p95, max] = percentiles(&mut self.latencies);
        tracing::info!(
            translated = self.latencies.len(),
            failed = self.failed,
            skipped = self.skipped,
            p50_ms = p50,
            p95_ms = p95,
            max_ms = max,
            "riepilogo traduzione"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use sv_llm::{BoxFuture, ChatResponse, LlmError, ModelInfo};

    use super::*;
    use crate::meeting::MeetingContext;

    /// Provider finto: "FAIL" nel testo → errore; altrimenti `[it] testo`.
    #[derive(Default)]
    struct FakeLlm {
        requests: Mutex<Vec<Vec<Message>>>,
    }

    impl LlmProvider for FakeLlm {
        fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<ChatResponse, LlmError>> {
            let text = req.messages.last().unwrap().content.clone();
            self.requests.lock().unwrap().push(req.messages);
            Box::pin(async move {
                if text.contains("FAIL") {
                    return Err(LlmError::Timeout);
                }
                Ok(ChatResponse {
                    content: format!("\"[it] {text}\""),
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

    fn job(id: u64, text: &str, language: Option<&str>) -> Job {
        Job {
            speaker: None,
            segment_id: id,
            text: text.into(),
            language: language.map(Into::into),
        }
    }

    async fn run_jobs(
        jobs: Vec<Job>,
        context: SharedContext,
    ) -> (Vec<TranslationEvent>, Arc<FakeLlm>) {
        let llm = Arc::new(FakeLlm::default());
        let out = Arc::new(Mutex::new(Vec::new()));
        let sink = out.clone();
        let (tx, task) = spawn(
            TranslatorConfig {
                llm: llm.clone(),
                model: "m".into(),
                target: Language::It,
                source_hint: None,
            },
            context,
            move |e| sink.lock().unwrap().push(e),
        );
        for j in jobs {
            tx.send(j).await.unwrap();
        }
        drop(tx);
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        let mut got = out.lock().unwrap().clone();
        got.sort_by_key(|e| e.segment_id);
        (got, llm)
    }

    #[tokio::test]
    async fn translates_by_id_and_skips_target_language() {
        let (got, llm) = run_jobs(
            vec![
                job(1, "Hello everyone", Some("en")),
                job(2, "ciao a tutti", Some("it")),
                job(3, "We ship next week", Some("en-US")),
                job(4, "...", Some("en")),
            ],
            SharedContext::default(),
        )
        .await;
        let pairs: Vec<(u64, &str)> = got
            .iter()
            .map(|e| (e.segment_id, e.text.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [(1, "[it] Hello everyone"), (3, "[it] We ship next week")]
        );
        // Niente chiamata per l'italiano e per il testo senza lettere.
        assert_eq!(llm.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn error_does_not_block_following_sentences() {
        let (got, _) = run_jobs(
            vec![
                job(1, "FAIL this", Some("en")),
                job(2, "this works", Some("en")),
            ],
            SharedContext::default(),
        )
        .await;
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].segment_id, 2);
    }

    #[tokio::test]
    async fn prompt_has_context_and_previous_sentences() {
        let context = SharedContext::new(MeetingContext {
            name: "Acme sync".into(),
            ..MeetingContext::default()
        });
        let (_, llm) = run_jobs(
            vec![
                job(1, "first one", Some("en")),
                job(2, "già detto", Some("it")),
                job(3, "third one", Some("en")),
            ],
            context,
        )
        .await;
        let requests = llm.requests.lock().unwrap();
        let last = requests
            .iter()
            .find(|m| m[1].content == "third one")
            .unwrap();
        assert!(last[0].content.contains("into Italian"));
        assert!(last[0].content.contains("MEETING CONTEXT"));
        assert!(last[0].content.contains("Name: Acme sync"));
        // Anche le frasi non tradotte fanno da contesto.
        assert!(last[0].content.contains("- first one\n- già detto"));
    }

    #[test]
    fn language_match() {
        assert!(same_language(Some("it"), Language::It));
        assert!(same_language(Some("EN-gb"), Language::En));
        assert!(!same_language(Some("en"), Language::It));
        assert!(!same_language(None, Language::It));
    }

    #[test]
    fn cleans_model_output() {
        assert_eq!(clean("<think>hmm</think>\n Ciao "), "Ciao");
        assert_eq!(clean("«Ciao a tutti»"), "Ciao a tutti");
        assert_eq!(clean("\"Ok\""), "Ok");
        assert_eq!(clean("L'ok è dato"), "L'ok è dato");
    }

    /// Prompt reale contro Ollama locale:
    /// `OLLAMA_MODEL=llama3.2:latest cargo test -p sottovoce -- --ignored ollama_live --nocapture`
    #[tokio::test]
    #[ignore]
    async fn ollama_live() {
        let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3.2:latest".into());
        let llm = sv_llm::OpenAiCompatible::new("http://localhost:11434/v1");
        let context = MeetingContext {
            name: "Release planning".into(),
            goal: "Decide the beta date".into(),
            notes: "QA = quality assurance team".into(),
            ..MeetingContext::default()
        }
        .prompt_block();
        let previous = vec!["Let's start with the release.".to_owned()];
        for text in [
            "We should ship the beta next week, but QA is behind.",
            "Can you send me the numbers by Friday?",
        ] {
            let req = ChatRequest::new(
                model.clone(),
                messages(Language::It, context.as_deref(), &previous, text),
            )
            .temperature(0.2)
            .timeout(Duration::from_secs(60));
            let resp = llm.chat(req).await.unwrap();
            println!(
                "{} ms | {text}\n   → {}",
                resp.latency.as_millis(),
                clean(&resp.content)
            );
        }
    }
}
