//! Chat con il modello durante la call.
//!
//! Domanda libera dell'utente ("cosa rispondo?", "dove sta questo simbolo?"):
//! il modello riceve contesto meeting, trascrizione recente e, se c'è, il
//! codice locale (vedi `codebase.rs`). La conversazione vive nel frontend;
//! qui si costruisce il prompt e si fa una singola chiamata.

use std::time::Duration;

use serde::Deserialize;
use sv_llm::Message;

use crate::meeting::MeetingContext;

/// Lunghezza massima della domanda (caratteri).
pub const MAX_QUESTION_CHARS: usize = 2_000;
/// Turni precedenti passati al modello.
const MAX_HISTORY_TURNS: usize = 8;
/// Caratteri massimi della cronologia.
const MAX_HISTORY_CHARS: usize = 6_000;
/// Caratteri massimi della trascrizione recente.
const MAX_TRANSCRIPT_CHARS: usize = 6_000;
/// Caratteri massimi per singolo turno della cronologia.
const MAX_TURN_CHARS: usize = 4_000;
/// Timeout della risposta: l'utente sta aspettando in call.
pub const TIMEOUT: Duration = Duration::from_secs(45);

const SYSTEM_PROMPT: &str = r#"You are Sottovoce, a concise assistant the User can talk to during a live meeting. Answer the User's questions using the meeting context, the live transcript and, when present, CODEBASE excerpts read from the User's local project.

Treat CODEBASE as ground truth: for code questions quote exact files, identifiers and behavior, and never invent code or APIs. If the available excerpts do not cover a point, say so instead of guessing.

Be brief and practical: a few sentences or a short list. Do not role-play other participants."#;

/// Ruolo di un turno della chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChatTurn {
    pub role: ChatRole,
    pub content: String,
}

pub struct ChatInput {
    pub question: String,
    pub history: Vec<ChatTurn>,
    pub transcript: String,
    /// Lingua della risposta, es. "Italian".
    pub language: &'static str,
    /// Blocco `CODEBASE` già costruito, se c'è una cartella di codice.
    pub code: Option<String>,
}

/// Messaggi per il provider: system con i materiali, cronologia, domanda.
pub fn messages(context: &MeetingContext, input: ChatInput) -> Vec<Message> {
    let mut system = String::from(SYSTEM_PROMPT);
    system.push_str("\n\nReply in ");
    system.push_str(input.language);
    system.push('.');
    if let Some(block) = context.prompt_block() {
        system.push_str("\n\n");
        system.push_str(&block);
    }
    let transcript = clip_tail(input.transcript.trim(), MAX_TRANSCRIPT_CHARS);
    if !transcript.is_empty() {
        system.push_str(
            "\n\nCALL SO FAR (live transcript, may be partial or contain recognition errors):\n",
        );
        system.push_str(&transcript);
    }
    if let Some(code) = input
        .code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        system.push_str("\n\n");
        system.push_str(code);
    }

    let mut messages = vec![Message::system(system)];
    let mut kept = Vec::new();
    let mut used = 0;
    for turn in input.history.iter().rev().take(MAX_HISTORY_TURNS) {
        let content = clip(turn.content.trim(), MAX_TURN_CHARS);
        if content.is_empty() {
            continue;
        }
        used += content.chars().count();
        if used > MAX_HISTORY_CHARS {
            break;
        }
        kept.push((turn.role, content));
    }
    kept.reverse();
    for (role, content) in kept {
        messages.push(match role {
            ChatRole::User => Message::user(content),
            ChatRole::Assistant => Message::assistant(content),
        });
    }
    messages.push(Message::user(clip(
        input.question.trim(),
        MAX_QUESTION_CHARS,
    )));
    messages
}

/// Primi `max` caratteri (non byte), con "…" se troncato.
pub(crate) fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max.saturating_sub(1)) {
        Some((i, _)) if s.chars().count() > max => format!("{}…", &s[..i]),
        _ => s.to_owned(),
    }
}

/// Ultimi `max` caratteri: per la trascrizione conta la coda, non l'inizio.
fn clip_tail(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_owned();
    }
    let start = s
        .char_indices()
        .nth(count - max)
        .map(|(i, _)| i)
        .unwrap_or(0);
    format!("…{}", &s[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> MeetingContext {
        MeetingContext {
            name: "Planning".into(),
            goal: "chiudere il budget".into(),
            ..MeetingContext::default()
        }
    }

    fn input() -> ChatInput {
        ChatInput {
            question: "Come chiudiamo il budget?".into(),
            history: vec![
                ChatTurn {
                    role: ChatRole::User,
                    content: "Riassumi.".into(),
                },
                ChatTurn {
                    role: ChatRole::Assistant,
                    content: "Serve la firma di Anna.".into(),
                },
            ],
            transcript: "[Io] Domani parlo con Anna.\n[Loro] Ok.".into(),
            language: "Italian",
            code: Some("CODEBASE\nFiles: 2\n".into()),
        }
    }

    #[test]
    fn system_gets_context_transcript_and_code() {
        let messages = messages(&context(), input());
        assert_eq!(messages[0].role, sv_llm::Role::System);
        let system = &messages[0].content;
        assert!(system.contains("Reply in Italian."));
        assert!(system.contains("MEETING CONTEXT"));
        assert!(system.contains("Goal: chiudere il budget"));
        assert!(system.contains("CALL SO FAR"));
        assert!(system.contains("[Io] Domani parlo con Anna."));
        assert!(system.contains("CODEBASE"));
        // Cronologia nell'ordine originale, poi la nuova domanda.
        assert_eq!(messages[1].role, sv_llm::Role::User);
        assert_eq!(messages[2].role, sv_llm::Role::Assistant);
        assert_eq!(messages[3].role, sv_llm::Role::User);
        assert_eq!(messages[3].content, "Come chiudiamo il budget?");
    }

    #[test]
    fn transcript_keeps_the_tail() {
        let long = format!("inizio\n{}", "x".repeat(10_000));
        let mut input = input();
        input.transcript = long;
        input.code = None;
        let system = &messages(&context(), input)[0].content;
        assert!(system.contains("…xxxx"), "la coda deve restare");
        assert!(!system.contains("inizio"), "l'inizio deve uscire");
    }

    #[test]
    fn empty_history_and_blank_question_are_safe() {
        let mut input = input();
        input.history = Vec::new();
        input.question = "   ".into();
        input.code = None;
        let messages = messages(&context(), input);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].content, "");
    }
}
