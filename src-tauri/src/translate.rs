//! Traduzione per frase dei `Final`: "Loro" nella lingua dell'utente, "Io"
//! nella lingua del meeting (impostata per "Loro" o rilevata dalle loro frasi),
//! così l'utente vede come dire in quella lingua ciò che ha appena detto.
//!
//! Un task riceve le frasi da una coda limitata e le traduce con al massimo
//! [`MAX_PARALLEL`] richieste LLM in parallelo. Le traduzioni possono
//! arrivare fuori ordine: la UI le associa alla frase tramite `segment_id`.
//! Un errore (LLM spento, timeout, ...) lascia la frase senza traduzione e
//! non ferma né la coda né la trascrizione.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use sv_audio::Channel;
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
/// Lingue rilevate di "Loro" considerate per la lingua del meeting.
const MEETING_LANGUAGE_WINDOW: usize = 6;

/// Lingua del meeting, in cui tradurre le frasi di "Io": quella impostata per
/// "Loro" se fissa, altrimenti la più frequente tra le ultime rilevate sulle
/// loro frasi (a parità vince la più recente). Condivisa tra sessione e
/// traduttore.
#[derive(Clone, Default)]
pub struct MeetingLanguage(Arc<Mutex<MeetingLanguageInner>>);

#[derive(Default)]
struct MeetingLanguageInner {
    fixed: Option<String>,
    recent: VecDeque<String>,
}

impl MeetingLanguage {
    /// `their_language` fissa (`It`/`En`) = lingua del meeting nota.
    pub fn new(their_language: Language) -> Self {
        let fixed = (their_language != Language::Multi).then(|| their_language.code().to_owned());
        Self(Arc::new(Mutex::new(MeetingLanguageInner {
            fixed,
            recent: VecDeque::with_capacity(MEETING_LANGUAGE_WINDOW),
        })))
    }

    /// Lingua rilevata su una frase di "Loro" (es. `en`, `en-US`).
    pub fn observe(&self, language: &str) {
        let Some(code) = base_code(language) else {
            return;
        };
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if inner.recent.len() == MEETING_LANGUAGE_WINDOW {
            inner.recent.pop_front();
        }
        inner.recent.push_back(code);
    }

    /// Codice della lingua del meeting; `None` finché non è nota.
    pub fn current(&self) -> Option<String> {
        let inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(fixed) = &inner.fixed {
            return Some(fixed.clone());
        }
        let mut best: Option<(&String, usize, usize)> = None;
        for (i, code) in inner.recent.iter().enumerate() {
            let count = inner.recent.iter().filter(|c| *c == code).count();
            if best.is_none_or(|(_, n, last)| count > n || (count == n && i > last)) {
                best = Some((code, count, i));
            }
        }
        best.map(|(code, _, _)| code.clone())
    }
}

/// Cosa serve per tradurre, preparato dal comando `start_session`.
#[derive(Clone)]
pub struct TranslatorConfig {
    pub llm: Arc<dyn LlmProvider>,
    pub model: String,
    /// Lingua di arrivo per "Loro" (concreta: mai `Multi`).
    pub target: Language,
    /// Lingua impostata per "Loro", usata se il motore non ne rileva una.
    pub source_hint: Option<Language>,
    /// Lingua impostata per "Io", usata se il motore non ne rileva una.
    pub my_hint: Option<Language>,
    /// Lingua di arrivo per "Io".
    pub meeting: MeetingLanguage,
}

impl TranslatorConfig {
    /// Lingua di arrivo e lingua di partenza presunta di una frase.
    fn languages(
        &self,
        channel: Channel,
        detected: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let (target, hint) = match channel {
            Channel::Them => (Some(self.target.code().to_owned()), self.source_hint),
            Channel::Me => (self.meeting.current(), self.my_hint),
        };
        let source = detected
            .map(str::to_owned)
            .or_else(|| hint.map(|l| l.code().to_owned()));
        (target, source)
    }

    /// Lingua di arrivo di una frase di `channel`; errore leggibile se non c'è.
    pub fn target_for(&self, channel: Channel) -> Result<String, String> {
        self.languages(channel, None)
            .0
            .filter(|code| language_name(code).is_some())
            .ok_or_else(|| "lingua del meeting non ancora nota: imposta la lingua degli altri nelle Impostazioni".into())
    }
}

/// Frase da tradurre.
#[derive(Debug, Clone)]
pub struct Job {
    pub segment_id: u64,
    pub channel: Channel,
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
    /// Lingua della traduzione (es. `it`, `en`).
    pub language: String,
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
        let (target, source) = cfg.languages(job.channel, job.language.as_deref());
        let target = target.filter(|t| {
            language_name(t).is_some()
                && !same_language(source.as_deref(), t)
                && has_letters(&job.text)
        });
        let previous: Vec<String> = history.iter().cloned().collect();
        if history.len() == HISTORY {
            history.pop_front();
        }
        let who = match job.channel {
            Channel::Me => "Io".to_owned(),
            Channel::Them => context.speaker_label(job.speaker),
        };
        history.push_back(format!("{who}: {}", job.text));
        let Some(target) = target else {
            stats.skipped += 1;
            continue;
        };

        // Attende uno slot: intanto le frasi successive restano in coda.
        let Ok(permit) = slots.clone().acquire_owned().await else {
            break;
        };
        let (cfg, emit) = (cfg.clone(), emit.clone());
        let block = context.prompt_block();
        inflight.spawn(async move {
            let _permit = permit;
            translate(&cfg, &target, block, &previous, job, emit.as_ref()).await
        });
    }

    while let Some(done) = inflight.join_next().await {
        stats.record(done.ok());
    }
    stats.report();
}

async fn translate(
    cfg: &TranslatorConfig,
    target: &str,
    context: Option<String>,
    previous: &[String],
    job: Job,
    emit: &(dyn Fn(TranslationEvent) + Send + Sync),
) -> Outcome {
    match translate_one(cfg, target, context.as_deref(), previous, &job.text).await {
        Ok((text, latency_ms)) => {
            emit(TranslationEvent {
                segment_id: job.segment_id,
                text,
                language: target.to_owned(),
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

/// Traduce una frase in `target` (codice lingua): (testo, latenza in ms).
/// Usata dal traduttore automatico e dal pulsante "Traduci".
pub async fn translate_one(
    cfg: &TranslatorConfig,
    target: &str,
    context: Option<&str>,
    previous: &[String],
    text: &str,
) -> Result<(String, u64), String> {
    let lang = language_name(target).ok_or_else(|| format!("lingua non supportata: {target}"))?;
    let req = ChatRequest::new(cfg.model.clone(), messages(lang, context, previous, text))
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

/// Prompt di traduzione verso `lang` (nome inglese della lingua): system con
/// istruzioni, contesto meeting e frasi precedenti; user con la sola frase.
pub fn messages(
    lang: &str,
    context: Option<&str>,
    previous: &[String],
    text: &str,
) -> Vec<Message> {
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

/// Nome inglese (per il prompt) delle lingue riconosciute dai motori STT;
/// `None` = lingua non supportata come arrivo.
pub fn language_name(code: &str) -> Option<&'static str> {
    Some(match base_code(code)?.as_str() {
        "it" => "Italian",
        "en" => "English",
        "es" => "Spanish",
        "fr" => "French",
        "de" => "German",
        "pt" => "Portuguese",
        "nl" => "Dutch",
        "ru" => "Russian",
        "ja" => "Japanese",
        "hi" => "Hindi",
        _ => return None,
    })
}

/// `en-US` → `en`; `None` se vuoto.
fn base_code(language: &str) -> Option<String> {
    let code = language
        .split(['-', '_'])
        .next()?
        .trim()
        .to_ascii_lowercase();
    (!code.is_empty()).then_some(code)
}

/// `language` (es. `en`, `en-US`) coincide con `target` (es. `en`).
fn same_language(language: Option<&str>, target: &str) -> bool {
    language
        .and_then(base_code)
        .is_some_and(|l| Some(l) == base_code(target))
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
            channel: Channel::Them,
            segment_id: id,
            text: text.into(),
            language: language.map(Into::into),
        }
    }

    fn mine(id: u64, text: &str, language: Option<&str>) -> Job {
        Job {
            channel: Channel::Me,
            ..job(id, text, language)
        }
    }

    async fn run_jobs(
        jobs: Vec<Job>,
        context: SharedContext,
    ) -> (Vec<TranslationEvent>, Arc<FakeLlm>) {
        run_with(jobs, context, MeetingLanguage::new(Language::Multi)).await
    }

    async fn run_with(
        jobs: Vec<Job>,
        context: SharedContext,
        meeting: MeetingLanguage,
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
                my_hint: Some(Language::It),
                meeting,
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
        assert!(
            last[0]
                .content
                .contains("- Loro: first one\n- Loro: già detto")
        );
    }

    #[tokio::test]
    async fn my_sentences_go_to_the_meeting_language() {
        let meeting = MeetingLanguage::new(Language::Multi);
        // Lingua del meeting ignota: nessuna traduzione di "Io".
        let (got, llm) = run_with(
            vec![mine(1, "ciao a tutti", Some("it"))],
            SharedContext::default(),
            meeting.clone(),
        )
        .await;
        assert!(got.is_empty());
        assert!(llm.requests.lock().unwrap().is_empty());

        meeting.observe("en-US");
        let (got, llm) = run_with(
            vec![
                mine(2, "possiamo spostare la demo?", None),
                mine(3, "Sounds good", Some("en")),
            ],
            SharedContext::default(),
            meeting,
        )
        .await;
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].segment_id, got[0].language.as_str()), (2, "en"));
        let requests = llm.requests.lock().unwrap();
        assert!(requests[0][0].content.contains("into English"));
    }

    #[test]
    fn meeting_language_fixed_or_most_frequent() {
        assert_eq!(
            MeetingLanguage::new(Language::En).current().as_deref(),
            Some("en")
        );
        let m = MeetingLanguage::new(Language::Multi);
        assert_eq!(m.current(), None);
        for l in ["en", "it", "en-GB", "it"] {
            m.observe(l);
        }
        // Parità: vince la più recente.
        assert_eq!(m.current().as_deref(), Some("it"));
        m.observe("EN");
        assert_eq!(m.current().as_deref(), Some("en"));
        // Finestra limitata: le vecchie escono.
        for _ in 0..MEETING_LANGUAGE_WINDOW {
            m.observe("es");
        }
        assert_eq!(m.current().as_deref(), Some("es"));
        m.observe("");
        assert_eq!(m.current().as_deref(), Some("es"));
    }

    #[test]
    fn language_match() {
        assert!(same_language(Some("it"), "it"));
        assert!(same_language(Some("EN-gb"), "en"));
        assert!(!same_language(Some("en"), "it"));
        assert!(!same_language(None, "it"));
        assert_eq!(language_name("pt-BR"), Some("Portuguese"));
        assert_eq!(language_name("multi"), None);
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
                messages("Italian", context.as_deref(), &previous, text),
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
