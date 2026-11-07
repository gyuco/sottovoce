//! Filtro anti-eco: senza cuffie il microfono ("Io") risente l'audio di
//! sistema ("Loro") dalle casse e lo trascrive una seconda volta.
//!
//! Le frasi finali di "Loro" restano qui per [`RECENT`]; una frase finale di
//! "Io" che le ricalca (stesse parole nello stesso ordine) è eco e si scarta.
//! Se "Loro" sta ancora parlando, la frase di "Io" aspetta al massimo
//! [`WAIT`] il `Final` corrispondente, che può arrivare dopo (diarization).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::sync::Notify;

/// Per quanto una frase di "Loro" può tornare come eco.
const RECENT: Duration = Duration::from_secs(20);
/// "Loro" è attivo se ha emesso qualcosa entro questo intervallo.
const ACTIVE: Duration = Duration::from_secs(3);
/// Attesa massima del `Final` di "Loro" per una frase di "Io".
const WAIT: Duration = Duration::from_secs(4);
/// Sotto questa lunghezza (parole) non si giudica: troppi falsi positivi.
const MIN_WORDS: usize = 3;
/// Quota di parole di "Io" ritrovate, in ordine, nelle frasi di "Loro".
const MATCH_RATIO: f32 = 0.75;

#[derive(Default)]
pub struct EchoGuard {
    state: Mutex<State>,
    notify: Notify,
}

#[derive(Default)]
struct State {
    /// Frasi finali recenti di "Loro" (arrivo, parole normalizzate).
    recent: VecDeque<(Instant, Vec<String>)>,
    /// Ultimo evento (parziale o finale) di "Loro".
    last_activity: Option<Instant>,
}

impl EchoGuard {
    /// "Loro" sta parlando (parziale).
    pub fn them_activity(&self) {
        self.lock().last_activity = Some(Instant::now());
    }

    /// Frase finale di "Loro".
    pub fn them_final(&self, text: &str) {
        let words = words(text);
        let now = Instant::now();
        {
            let mut state = self.lock();
            state.last_activity = Some(now);
            if !words.is_empty() {
                state.recent.push_back((now, words));
            }
            prune(&mut state.recent, now);
        }
        self.notify.notify_waiters();
    }

    /// `true` se la frase di "Io" è eco di "Loro". Può attendere fino a
    /// [`WAIT`] se "Loro" è attivo ma la sua frase non è ancora finale.
    pub async fn is_echo(&self, text: &str) -> bool {
        let mine = words(text);
        if mine.len() < MIN_WORDS {
            return false;
        }
        let deadline = Instant::now() + WAIT;
        loop {
            // Registrati prima del controllo: nessuna notifica persa.
            let notified = self.notify.notified();
            let active = {
                let mut state = self.lock();
                let now = Instant::now();
                prune(&mut state.recent, now);
                if matches(&mine, state.recent.iter().map(|(_, w)| w)) {
                    return true;
                }
                state
                    .last_activity
                    .is_some_and(|t| now.duration_since(t) < ACTIVE)
            };
            let now = Instant::now();
            if !active || now >= deadline {
                tracing::debug!(
                    text,
                    them_active = active,
                    recent = self.lock().recent.len(),
                    "frase di \"Io\" tenuta: nessuna frase di \"Loro\" corrispondente"
                );
                return false;
            }
            let _ = tokio::time::timeout(deadline - now, notified).await;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn prune(recent: &mut VecDeque<(Instant, Vec<String>)>, now: Instant) {
    while recent
        .front()
        .is_some_and(|(t, _)| now.duration_since(*t) > RECENT)
    {
        recent.pop_front();
    }
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Le parole di `mine` compaiono in ordine nel testo recente di "Loro"
/// (sottosequenza comune più lunga ≥ [`MATCH_RATIO`]): l'eco può perdere o
/// storpiare qualche parola, ma non cambia l'ordine.
fn matches<'a>(mine: &[String], theirs: impl Iterator<Item = &'a Vec<String>>) -> bool {
    let theirs: Vec<&String> = theirs.flatten().collect();
    if theirs.is_empty() {
        return false;
    }
    let mut prev = vec![0usize; theirs.len() + 1];
    let mut cur = vec![0usize; theirs.len() + 1];
    for m in mine {
        for (j, t) in theirs.iter().enumerate() {
            cur[j + 1] = if m == *t {
                prev[j] + 1
            } else {
                cur[j].max(prev[j + 1])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let common = prev[theirs.len()];
    common as f32 >= MATCH_RATIO * mine.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard_with(texts: &[&str]) -> EchoGuard {
        let guard = EchoGuard::default();
        for t in texts {
            guard.them_final(t);
        }
        guard
    }

    #[tokio::test]
    async fn exact_echo_is_dropped() {
        let g = guard_with(&["Thanks everyone. Let's review the action items from last week."]);
        assert!(
            g.is_echo("Thanks everyone. Let's review the action items from last week.")
                .await
        );
    }

    #[tokio::test]
    async fn partial_and_noisy_echo_is_dropped() {
        let g = guard_with(&[
            "Thanks everyone.",
            "Let's review the action items from last week.",
        ]);
        assert!(
            g.is_echo("everyone let's review the actions items from last week")
                .await
        );
    }

    #[tokio::test]
    async fn own_reply_is_kept() {
        let g = guard_with(&["Do you agree with the plan?"]);
        assert!(
            !g.is_echo("Yes, I agree with the plan, but not on timing.")
                .await
        );
        assert!(!g.is_echo("Mi sentite?").await);
    }

    #[tokio::test]
    async fn waits_for_late_final_of_them() {
        let g = std::sync::Arc::new(EchoGuard::default());
        g.them_activity();
        let late = g.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            late.them_final("Let's review the action items from last week.");
        });
        assert!(g.is_echo("review the action items from last week").await);
    }

    #[tokio::test]
    async fn no_wait_when_them_silent() {
        let g = EchoGuard::default();
        let t = Instant::now();
        assert!(!g.is_echo("review the action items from last week").await);
        assert!(t.elapsed() < Duration::from_millis(100));
    }
}
