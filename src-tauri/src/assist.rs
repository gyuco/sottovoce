//! Live assist: ogni [`PERIOD`] suggerisce all'utente cosa dire o chiedere.
//!
//! Il task riceve le frasi finali di entrambi i canali e, a ogni ciclo, se
//! sono arrivate frasi nuove, chiede al modello `{"say_this": [...],
//! "ask_this": [...]}`. Niente frasi nuove = niente chiamata (silenzio
//! gratis). La richiesta è fatta dentro il ciclo: se è lenta, i tick persi
//! vengono saltati invece di accumularsi. I suggerimenti simili a quelli già
//! mostrati nella sessione vengono scartati.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sv_audio::Channel;
use sv_llm::{ChatRequest, LlmProvider, Message, parse_json};
use sv_stt::Language;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::meeting::SharedContext;
use crate::session::percentiles;

/// Intervallo fra due cicli.
pub const PERIOD: Duration = Duration::from_secs(20);
/// Frasi in attesa fra due cicli (oltre si scartano, con log).
pub const QUEUE: usize = 256;
/// Frasi precedenti al ciclo passate come contesto.
const EARLIER_LINES: usize = 20;
/// Suggerimenti già mostrati elencati nel prompt ("non ripetere").
const SHOWN_IN_PROMPT: usize = 15;
const MAX_PER_KIND: usize = 3;
/// Soglia di similarità (Jaccard sui token) oltre cui è un duplicato.
const DUP_THRESHOLD: f64 = 0.6;
const TIMEOUT: Duration = Duration::from_secs(20);

const SYSTEM_PROMPT: &str = r#"You are a live meeting coach. You receive a rolling 20-second transcript from an ongoing meeting. Your job is to surface helpful nudges for the User.

All assistance must be directed to the "User" (labeled [Io]). Do NOT provide assistance for other participants (labeled [Loro], [Speaker N] or by name). Focus exclusively on what the User can say or ask.

You may receive MEETING CONTEXT (name, purpose, notes, goals): keep suggestions aligned with it.

Surface: a way to respond to something said; an opportunity to summarize or steer; a topic to park; a decision to capture; a commitment to acknowledge; a clarifying question; a vague claim to pin down; follow-up questions.

Only surface assists when the User would genuinely benefit. Otherwise return empty arrays.

Output JSON only:
{"say_this": ["..."], "ask_this": ["..."]}

Rules: 0-3 items per array; every suggestion must connect to the transcript; ready-to-use first-person lines; write them in the meeting language (and add the translation in the user's language if different)."#;

pub struct AssistConfig {
    pub llm: Arc<dyn LlmProvider>,
    pub model: String,
    /// Lingua dell'utente, per l'eventuale traduzione dei suggerimenti.
    pub user_language: Language,
}

/// Frase finale passata dal pump della sessione.
#[derive(Debug, Clone)]
pub struct AssistLine {
    pub channel: Channel,
    pub text: String,
    /// Parlante su "Loro", se riconosciuto.
    pub speaker: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AssistKind {
    Say,
    Ask,
}

impl AssistKind {
    /// Etichetta salvata nel database (`say`/`ask`).
    pub fn code(self) -> &'static str {
        match self {
            Self::Say => "say",
            Self::Ask => "ask",
        }
    }
}

/// Suggerimento inviato alla UI. `at` = ms dall'avvio della sessione.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistEvent {
    pub id: u64,
    pub kind: AssistKind,
    pub text: String,
    pub at: u64,
}

#[derive(Debug, Default, Deserialize)]
struct AssistOutput {
    #[serde(default)]
    say_this: Vec<String>,
    #[serde(default)]
    ask_this: Vec<String>,
}

/// Avvia il task. Chiudere il `Sender` (drop) lo ferma.
pub fn spawn(
    cfg: AssistConfig,
    context: SharedContext,
    started: Instant,
    on_assist: impl Fn(AssistEvent) + Send + Sync + 'static,
) -> (mpsc::Sender<AssistLine>, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(QUEUE);
    let task = tokio::spawn(run(cfg, context, started, PERIOD, rx, on_assist));
    (tx, task)
}

async fn run(
    cfg: AssistConfig,
    context: SharedContext,
    started: Instant,
    period: Duration,
    mut rx: mpsc::Receiver<AssistLine>,
    emit: impl Fn(AssistEvent),
) {
    let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut state = State::default();

    loop {
        tokio::select! {
            line = rx.recv() => match line {
                Some(line) => state.push(line),
                None => break,
            },
            _ = ticks.tick() => {
                let Some(messages) = state.next_prompt(&cfg, &context) else {
                    state.stats.idle += 1;
                    continue;
                };
                let at = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                for event in state.ask(&cfg, messages, at).await {
                    emit(event);
                }
            }
        }
    }
    state.stats.report();
}

#[derive(Default)]
struct State {
    /// Frasi già usate in cicli precedenti (contesto).
    earlier: VecDeque<AssistLine>,
    /// Frasi arrivate dopo l'ultimo ciclo.
    fresh: Vec<AssistLine>,
    shown: Vec<String>,
    next_id: u64,
    stats: Stats,
}

impl State {
    fn push(&mut self, line: AssistLine) {
        if !line.text.trim().is_empty() {
            self.fresh.push(line);
        }
    }

    /// Prompt del ciclo; `None` se non ci sono frasi nuove. Le frasi nuove
    /// passano nel contesto dei cicli successivi.
    fn next_prompt(&mut self, cfg: &AssistConfig, context: &SharedContext) -> Option<Vec<Message>> {
        if self.fresh.is_empty() {
            return None;
        }
        let fresh = std::mem::take(&mut self.fresh);
        let shown_from = self.shown.len().saturating_sub(SHOWN_IN_PROMPT);
        // Etichette risolte ora: un nome dato a un parlante vale anche per
        // le sue frasi precedenti.
        let label = |speaker| context.speaker_label(speaker);
        let messages = messages(
            cfg.user_language,
            context.prompt_block().as_deref(),
            &self.shown[shown_from..],
            self.earlier.make_contiguous(),
            &fresh,
            &label,
        );
        for line in fresh {
            if self.earlier.len() == EARLIER_LINES {
                self.earlier.pop_front();
            }
            self.earlier.push_back(line);
        }
        Some(messages)
    }

    async fn ask(
        &mut self,
        cfg: &AssistConfig,
        messages: Vec<Message>,
        at: u64,
    ) -> Vec<AssistEvent> {
        let req = ChatRequest::new(cfg.model.clone(), messages)
            .json()
            .temperature(0.3)
            .timeout(TIMEOUT);
        let resp = match cfg.llm.chat(req).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("live assist: richiesta fallita: {e}");
                self.stats.failed += 1;
                return Vec::new();
            }
        };
        self.stats
            .latencies
            .push(u64::try_from(resp.latency.as_millis()).unwrap_or(u64::MAX));
        let out: AssistOutput = match parse_json(&resp.content) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!("live assist: JSON non valido, ciclo saltato: {e}");
                self.stats.failed += 1;
                return Vec::new();
            }
        };

        let mut events = Vec::new();
        for (kind, items) in [
            (AssistKind::Say, out.say_this),
            (AssistKind::Ask, out.ask_this),
        ] {
            for text in items.into_iter().take(MAX_PER_KIND) {
                let text = text.trim().to_owned();
                if text.is_empty() {
                    continue;
                }
                if self.shown.iter().any(|s| similar(s, &text)) {
                    self.stats.duplicates += 1;
                    continue;
                }
                self.next_id += 1;
                self.shown.push(text.clone());
                events.push(AssistEvent {
                    id: self.next_id,
                    kind,
                    text,
                    at,
                });
            }
        }
        self.stats.suggestions += events.len();
        events
    }
}

/// Prompt del live assist: system da `docs/DESIGN.md` §6 più lingua
/// dell'utente, contesto meeting e suggerimenti già dati; user con la
/// trascrizione etichettata.
fn messages(
    user_language: Language,
    context: Option<&str>,
    shown: &[String],
    earlier: &[AssistLine],
    fresh: &[AssistLine],
    label: &dyn Fn(Option<u8>) -> String,
) -> Vec<Message> {
    let mut system = String::from(SYSTEM_PROMPT);
    let lang = match user_language {
        Language::En => "English",
        Language::It | Language::Multi => "Italian",
    };
    system.push_str(&format!("\n\nThe User's language is {lang}."));
    if let Some(block) = context {
        system.push_str("\n\n");
        system.push_str(block);
    }
    if !shown.is_empty() {
        system.push_str("\n\nAlready suggested (do not repeat or rephrase):");
        for s in shown {
            system.push_str("\n- ");
            system.push_str(s);
        }
    }

    let mut user = String::new();
    if !earlier.is_empty() {
        user.push_str("Earlier in the meeting:\n");
        push_lines(&mut user, earlier, label);
        user.push('\n');
    }
    user.push_str("Last 20 seconds:\n");
    push_lines(&mut user, fresh, label);
    vec![Message::system(system), Message::user(user.trim_end())]
}

fn push_lines(out: &mut String, lines: &[AssistLine], label: &dyn Fn(Option<u8>) -> String) {
    for l in lines {
        match l.channel {
            Channel::Me => out.push_str("[Io]"),
            Channel::Them => {
                out.push('[');
                out.push_str(&label(l.speaker));
                out.push(']');
            }
        }
        out.push(' ');
        out.push_str(l.text.trim());
        out.push('\n');
    }
}

/// Token normalizzati: minuscole, solo lettere e cifre.
fn tokens(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Jaccard sui token sopra [`DUP_THRESHOLD`].
fn similar(a: &str, b: &str) -> bool {
    let (a, b) = (tokens(a), tokens(b));
    if a.is_empty() || b.is_empty() {
        return a == b;
    }
    let inter = a.intersection(&b).count() as f64;
    let union = a.union(&b).count() as f64;
    inter / union > DUP_THRESHOLD
}

#[derive(Default)]
struct Stats {
    latencies: Vec<u64>,
    idle: usize,
    failed: usize,
    suggestions: usize,
    duplicates: usize,
}

impl Stats {
    fn report(mut self) {
        let [p50, p95, max] = percentiles(&mut self.latencies);
        tracing::info!(
            calls = self.latencies.len() + self.failed,
            failed = self.failed,
            idle_cycles = self.idle,
            suggestions = self.suggestions,
            duplicates = self.duplicates,
            p50_ms = p50,
            p95_ms = p95,
            max_ms = max,
            "riepilogo live assist"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use sv_llm::{BoxFuture, ChatResponse, LlmError, ModelInfo};

    use super::*;
    use crate::meeting::MeetingContext;

    /// Risponde con le risposte in coda, in ordine; registra le richieste.
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
            assert!(req.json, "il live assist chiede JSON");
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

    fn line(channel: Channel, text: &str) -> AssistLine {
        AssistLine {
            channel,
            text: text.into(),
            speaker: None,
        }
    }

    struct Harness {
        tx: mpsc::Sender<AssistLine>,
        task: JoinHandle<()>,
        events: Arc<Mutex<Vec<AssistEvent>>>,
    }

    fn start(llm: Arc<ScriptedLlm>, context: SharedContext) -> Harness {
        let (tx, rx) = mpsc::channel(QUEUE);
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let cfg = AssistConfig {
            llm,
            model: "m".into(),
            user_language: Language::It,
        };
        let task = tokio::spawn(run(cfg, context, Instant::now(), PERIOD, rx, move |e| {
            sink.lock().unwrap().push(e)
        }));
        Harness { tx, task, events }
    }

    async fn cycle() {
        tokio::time::sleep(PERIOD + Duration::from_millis(10)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn no_call_without_new_lines_and_dedup_across_cycles() {
        let llm = ScriptedLlm::new(&[
            r#"{"say_this": ["Proposal: we fix the date on Friday."], "ask_this": ["Who owns QA?"]}"#,
            // Seconda risposta: un quasi-duplicato e una novità.
            "Ecco:\n```json\n{\"say_this\": [\"Proposal: we fix the date on Friday\"], \"ask_this\": [\"What is the budget?\"]}\n```",
        ]);
        let h = start(llm.clone(), SharedContext::default());

        cycle().await; // silenzio: nessuna chiamata
        assert_eq!(llm.requests.lock().unwrap().len(), 0);

        h.tx.send(line(Channel::Them, "We could ship next week."))
            .await
            .unwrap();
        h.tx.send(line(Channel::Me, "Not sure QA is ready."))
            .await
            .unwrap();
        cycle().await;
        cycle().await; // di nuovo silenzio
        assert_eq!(llm.requests.lock().unwrap().len(), 1);

        h.tx.send(line(Channel::Them, "And the budget?"))
            .await
            .unwrap();
        cycle().await;
        drop(h.tx);
        h.task.await.unwrap();

        let events = h.events.lock().unwrap();
        let got: Vec<(AssistKind, &str)> =
            events.iter().map(|e| (e.kind, e.text.as_str())).collect();
        assert_eq!(
            got,
            [
                (AssistKind::Say, "Proposal: we fix the date on Friday."),
                (AssistKind::Ask, "Who owns QA?"),
                (AssistKind::Ask, "What is the budget?"),
            ]
        );
        let ids: Vec<u64> = events.iter().map(|e| e.id).collect();
        assert_eq!(ids, [1, 2, 3]);

        let requests = llm.requests.lock().unwrap();
        // Secondo ciclo: frasi precedenti come contesto, già mostrati nel prompt.
        let user = &requests[1][1].content;
        assert!(user.contains(
            "Earlier in the meeting:\n[Loro] We could ship next week.\n[Io] Not sure QA is ready."
        ));
        assert!(user.ends_with("Last 20 seconds:\n[Loro] And the budget?"));
        assert!(requests[1][0].content.contains(
            "do not repeat or rephrase):\n- Proposal: we fix the date on Friday.\n- Who owns QA?"
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn invalid_json_skips_cycle_and_context_is_in_prompt() {
        let llm = ScriptedLlm::new(&["Non so cosa suggerire.", r#"{"ask_this": ["Quando?"]}"#]);
        let context = SharedContext::new(MeetingContext {
            goal: "chiudere il contratto".into(),
            ..MeetingContext::default()
        });
        let h = start(llm.clone(), context);
        h.tx.send(line(Channel::Them, "Ciao")).await.unwrap();
        cycle().await;
        h.tx.send(line(Channel::Them, "Firmiamo?")).await.unwrap();
        cycle().await;
        drop(h.tx);
        h.task.await.unwrap();

        let events = h.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].text, "Quando?");
        let requests = llm.requests.lock().unwrap();
        assert!(
            requests[0][0]
                .content
                .contains("Goal: chiudere il contratto")
        );
        assert!(
            requests[0][0]
                .content
                .contains("The User's language is Italian.")
        );
    }

    #[test]
    fn similarity() {
        assert!(similar("Ask about the budget!", "ask about the BUDGET"));
        assert!(!similar(
            "Ask about the budget",
            "Confirm the deadline with Marco"
        ));
        assert!(similar(
            "Proposta: chiudiamo venerdì",
            "proposta chiudiamo venerdì."
        ));
    }

    #[tokio::test]
    async fn caps_items_per_kind_and_dedups_within_reply() {
        let llm = ScriptedLlm::new(&[
            r#"{"say_this": ["Alpha one", "Beta two", "alpha one!", "Gamma three", "Delta four"]}"#,
        ]);
        let cfg = AssistConfig {
            llm,
            model: "m".into(),
            user_language: Language::It,
        };
        let mut state = State::default();
        let events = state.ask(&cfg, vec![Message::user("x")], 0).await;
        let texts: Vec<&str> = events.iter().map(|e| e.text.as_str()).collect();
        // Primi 3 della risposta, il duplicato interno scartato.
        assert_eq!(texts, ["Alpha one", "Beta two"]);
        assert_eq!(state.stats.duplicates, 1);
    }

    /// Prompt reale contro Ollama locale:
    /// `OLLAMA_MODEL=llama3.2:latest cargo test -p sottovoce -- --ignored assist_ollama_live --nocapture`
    #[tokio::test]
    #[ignore]
    async fn assist_ollama_live() {
        let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3.2:latest".into());
        let cfg = AssistConfig {
            llm: Arc::new(sv_llm::OpenAiCompatible::new("http://localhost:11434/v1")),
            model,
            user_language: Language::It,
        };
        let context = MeetingContext {
            name: "Release planning".into(),
            goal: "Fissare la data della beta".into(),
            ..MeetingContext::default()
        };
        let mut state = State::default();
        for (ch, text) in [
            (Channel::Them, "We could ship the beta next week."),
            (Channel::Me, "I'm not sure QA is ready."),
            (
                Channel::Them,
                "QA says two more days, but marketing wants Monday.",
            ),
        ] {
            state.push(line(ch, text));
        }
        let messages = state
            .next_prompt(&cfg, &SharedContext::new(context))
            .unwrap();
        let started = Instant::now();
        let events = state.ask(&cfg, messages, 0).await;
        println!(
            "{} ms, stats failed={}",
            started.elapsed().as_millis(),
            state.stats.failed
        );
        for e in events {
            println!("{:?}: {}", e.kind, e.text);
        }
    }
}
