//! Domande sullo storico: "cosa avevamo deciso con ACME sul rinnovo?".
//!
//! Il recupero sta in `search.rs` (passaggi semantici + per parole, contesto
//! numerato di `sv-search`); qui si costruiscono la query di ricerca e il
//! prompt, e si legge la risposta (citazioni, "non trovato"). La
//! conversazione vive nel frontend, come per la chat in call.

use std::time::Duration;

use serde::Serialize;
use sv_llm::Message;

use crate::chat::{ChatRole, ChatTurn, clip};
use crate::search::kind_name;

/// Lunghezza massima della domanda (caratteri).
pub const MAX_QUESTION_CHARS: usize = 2_000;
/// Turni precedenti passati al modello.
const MAX_HISTORY_TURNS: usize = 6;
/// Caratteri massimi della cronologia.
const MAX_HISTORY_CHARS: usize = 4_000;
/// Caratteri massimi per singolo turno della cronologia.
const MAX_TURN_CHARS: usize = 1_500;
/// Caratteri del turno precedente aggiunti alla query di ricerca.
const MAX_FOLLOWUP_CHARS: usize = 400;
/// Timeout della risposta.
pub const TIMEOUT: Duration = Duration::from_secs(45);
/// Risposta convenzionale del modello quando i passaggi non bastano.
const NOT_FOUND: &str = "NOT_FOUND";

const SYSTEM_PROMPT: &str = r#"You are Sottovoce, an assistant that answers the User's questions about their past meetings. Use ONLY the numbered PASSAGES below: transcript excerpts, 5-minute notes, summaries and action items from the User's saved meetings. Each passage starts with [n], the meeting name, its date and the time within the meeting.

Rules:
- Every statement must come from the passages. Cite the supporting passages inline with their numbers, like [2] or [1, 3].
- Never use outside knowledge and never guess names, dates, amounts, owners or decisions.
- If the passages do not contain the answer, reply with exactly NOT_FOUND and nothing else.
- If the passages answer only part of the question, answer that part and say plainly what is missing.
- Transcripts come from speech recognition and may contain errors. "Io" is the User; other labels are the other participants.
- Reply in the same language as the User's question, even when the passages are in another language.
- Be concise: a few sentences or a short list."#;

/// Query di ricerca: nei turni successivi ("e con Luca?") la domanda da
/// sola non basta, si aggiunge l'ultima domanda dell'utente.
pub fn search_query(question: &str, history: &[ChatTurn]) -> String {
    let question = question.trim();
    let previous = history
        .iter()
        .rev()
        .find(|t| t.role == ChatRole::User)
        .map(|t| clip(t.content.trim(), MAX_FOLLOWUP_CHARS));
    match previous {
        Some(p) if !p.is_empty() && p != question => format!("{p}\n{question}"),
        _ => question.to_owned(),
    }
}

/// Messaggi per il provider: regole + passaggi, cronologia (senza le
/// citazioni: i numeri cambiano a ogni domanda), domanda.
pub fn messages(question: &str, history: &[ChatTurn], context: &str, today: &str) -> Vec<Message> {
    let mut system = String::from(SYSTEM_PROMPT);
    system.push_str("\n\nToday is ");
    system.push_str(today);
    system.push_str(".\n\nPASSAGES:\n");
    system.push_str(context.trim_end());

    let mut messages = vec![Message::system(system)];
    let mut kept = Vec::new();
    let mut used = 0;
    for turn in history.iter().rev().take(MAX_HISTORY_TURNS) {
        let content = match turn.role {
            ChatRole::User => turn.content.trim().to_owned(),
            ChatRole::Assistant => sv_search::strip_citations(turn.content.trim()),
        };
        let content = clip(content.trim(), MAX_TURN_CHARS);
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
    messages.push(Message::user(clip(question.trim(), MAX_QUESTION_CHARS)));
    messages
}

/// Fonte citata nella risposta, per i link della UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySource {
    pub n: u32,
    pub meeting_id: i64,
    pub meeting_name: String,
    pub started_at_ms: i64,
    /// `transcript`, `block`, `overview`, `keyPoints`, `actionItems`.
    pub kind: &'static str,
    /// Prima frase del passaggio (solo trascrizione): per scorrere al punto.
    pub segment_id: Option<i64>,
    pub start_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryAnswer {
    /// Risposta con le citazioni `[n]`; vuota se `found` è falso.
    pub answer: String,
    /// `false`: nei meeting non c'è la risposta (la UI mostra il messaggio).
    pub found: bool,
    /// La ricerca per significato ha partecipato; `false` = solo parole.
    pub semantic: bool,
    /// Fonti citate, nell'ordine della prima citazione.
    pub sources: Vec<HistorySource>,
}

/// Risposta "non trovato": senza passaggi non si chiama il modello.
pub fn not_found(semantic: bool) -> HistoryAnswer {
    HistoryAnswer {
        answer: String::new(),
        found: false,
        semantic,
        sources: Vec::new(),
    }
}

/// Legge la risposta del modello: "non trovato" o testo con le fonti citate.
pub fn answer(reply: &str, sources: &[sv_search::Source], semantic: bool) -> HistoryAnswer {
    let reply = reply.trim();
    let cited = sv_search::citations(reply);
    let bare = reply.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
    if bare == NOT_FOUND || (reply.contains(NOT_FOUND) && cited.is_empty()) {
        return not_found(semantic);
    }
    let text = reply.replace(NOT_FOUND, "").trim().to_owned();
    let sources = cited
        .iter()
        .filter_map(|n| sources.iter().find(|s| s.n == *n))
        .map(|s| {
            let (kind, segment_id) = kind_name(s.source);
            HistorySource {
                n: s.n,
                meeting_id: s.meeting_id,
                meeting_name: s.meeting_name.clone(),
                started_at_ms: s.started_at_ms,
                kind,
                segment_id,
                start_ms: s.start_ms,
                text: s.text.clone(),
            }
        })
        .collect();
    HistoryAnswer {
        answer: text,
        found: true,
        semantic,
        sources,
    }
}

#[cfg(test)]
mod tests {
    use sv_search::ChunkRef;

    use super::*;

    fn turn(role: ChatRole, content: &str) -> ChatTurn {
        ChatTurn {
            role,
            content: content.into(),
        }
    }

    fn source(n: u32, first: i64) -> sv_search::Source {
        sv_search::Source {
            n,
            meeting_id: 7,
            meeting_name: "ACME".into(),
            started_at_ms: 1,
            source: ChunkRef::Transcript {
                first,
                last: first + 1,
            },
            start_ms: 61_000,
            text: "Anna: rinnoviamo".into(),
        }
    }

    #[test]
    fn follow_up_query_carries_the_previous_question() {
        let history = vec![
            turn(ChatRole::User, "Cosa abbiamo deciso con ACME?"),
            turn(ChatRole::Assistant, "Rinnovo a marzo [1]."),
        ];
        assert_eq!(
            search_query(" e con Luca? ", &history),
            "Cosa abbiamo deciso con ACME?\ne con Luca?"
        );
        assert_eq!(search_query("Budget?", &[]), "Budget?");
    }

    #[test]
    fn prompt_has_rules_passages_and_clean_history() {
        let history = vec![
            turn(ChatRole::User, "Chi manda il preventivo?"),
            turn(ChatRole::Assistant, "Lo manda Luca [2]."),
        ];
        let m = messages(
            "E la scadenza?",
            &history,
            "[1] ACME · 2026-11-02 · 01:00 · Anna: ok\n",
            "2026-11-09",
        );
        assert_eq!(m.len(), 4);
        assert!(m[0].content.contains("NOT_FOUND"));
        assert!(
            m[0].content
                .contains("same language as the User's question")
        );
        assert!(m[0].content.contains("Today is 2026-11-09."));
        assert!(
            m[0].content
                .ends_with("PASSAGES:\n[1] ACME · 2026-11-02 · 01:00 · Anna: ok")
        );
        assert_eq!(m[2].role, sv_llm::Role::Assistant);
        assert_eq!(m[2].content, "Lo manda Luca.");
        assert_eq!(m[3].content, "E la scadenza?");
    }

    #[test]
    fn history_is_limited_to_the_last_turns() {
        let history: Vec<ChatTurn> = (0..10)
            .map(|i| turn(ChatRole::User, &format!("domanda {i}")))
            .collect();
        let m = messages("ultima", &history, "", "2026-11-09");
        assert_eq!(m.len(), 1 + MAX_HISTORY_TURNS + 1);
        assert_eq!(m[1].content, "domanda 4");
    }

    #[test]
    fn answer_keeps_cited_sources_in_order() {
        let sources = [source(1, 10), source(2, 20), source(3, 30)];
        let a = answer(
            "Rinnovo a marzo [3], firma di Anna [1, 3]. Fonte [9] ignota.",
            &sources,
            true,
        );
        assert!(a.found && a.semantic);
        let ns: Vec<u32> = a.sources.iter().map(|s| s.n).collect();
        assert_eq!(ns, vec![3, 1]);
        assert_eq!(a.sources[0].kind, "transcript");
        assert_eq!(a.sources[0].segment_id, Some(30));
    }

    #[test]
    fn not_found_is_recognized() {
        let sources = [source(1, 10)];
        for reply in ["NOT_FOUND", " `NOT_FOUND`. ", "Mi dispiace: NOT_FOUND"] {
            let a = answer(reply, &sources, false);
            assert!(!a.found, "{reply}");
            assert!(a.answer.is_empty() && a.sources.is_empty());
        }
        // Risposta parziale con citazioni: resta valida.
        assert!(answer("Il budget è 10k [1]. NOT_FOUND", &sources, true).found);
        assert!(!not_found(true).found);
    }
}
