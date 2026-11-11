//! Filtro anti-eco: senza cuffie il microfono ("Io") risente l'audio di
//! sistema ("Loro") dalle casse e lo trascrive una seconda volta.
//!
//! Le frasi finali di "Loro" restano qui per [`RECENT`] con i loro tempi; una
//! frase finale di "Io" che le ricalca (stesse parole nello stesso ordine) è
//! eco e si scarta. Le frasi corte (meno di [`MIN_WORDS`] parole) si
//! giudicano solo contro le frasi di "Loro" sovrapposte nel tempo.
//!
//! Se mentre "Io" parlava anche l'audio di sistema aveva voce, la frase di
//! "Loro" corrispondente può non essere ancora arrivata (decodifica,
//! diarization): la frase di "Io" la aspetta al massimo [`WAIT`]. L'attività
//! di "Loro" si misura sull'audio, non sugli eventi STT: il motore locale
//! emette un solo parziale a inizio frase e la diarization ritarda i `Final`.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sv_audio::AudioFrame;
use tokio::sync::Notify;

/// Per quanto una frase di "Loro" può tornare come eco.
const RECENT: Duration = Duration::from_secs(20);
/// Attesa massima del `Final` di "Loro" per una frase di "Io" (decodifica
/// + diarization, che trattiene il `Final` fino a 2 s).
const WAIT: Duration = Duration::from_secs(6);
/// Sotto questa lunghezza (parole) il confronto vale solo con le frasi di
/// "Loro" sovrapposte nel tempo: troppi falsi positivi altrimenti.
const MIN_WORDS: usize = 3;
/// Quota di parole di "Io" ritrovate, in ordine, nelle frasi di "Loro".
const MATCH_RATIO: f32 = 0.7;
/// Frase corta: quota della sua durata coperta da una frase di "Loro".
const SHORT_OVERLAP: f32 = 0.5;
/// Livello RMS sopra cui l'audio di sistema ha voce (circa -46 dBFS).
const LOUD_RMS: f32 = 0.005;
/// Quota della frase di "Io" con audio di sistema attivo per sospettare eco.
const LOUD_SHARE: f32 = 0.3;
/// Storia dell'audio di sistema conservata (tempo di cattura).
const AUDIO_HISTORY: Duration = Duration::from_secs(60);
/// Audio chiuso (stop): attesa residua massima, dentro lo stop della sessione.
const WAIT_AFTER_CLOSE: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct EchoGuard {
    state: Mutex<State>,
    notify: Notify,
}

struct TheirFinal {
    arrived: Instant,
    start: Duration,
    end: Duration,
    words: Vec<String>,
}

#[derive(Default)]
struct State {
    /// Frasi finali recenti di "Loro".
    recent: VecDeque<TheirFinal>,
    /// Audio di sistema a tratti omogenei: (inizio, fine, con voce).
    audio: VecDeque<(Duration, Duration, bool)>,
    /// Fine dell'audio di sistema (stop della cattura).
    closed: Option<Instant>,
}

impl EchoGuard {
    /// Frame dell'audio di sistema ("Loro").
    pub fn them_audio(&self, frame: &AudioFrame) {
        if frame.samples.is_empty() {
            return;
        }
        let rms =
            (frame.samples.iter().map(|s| s * s).sum::<f32>() / frame.samples.len() as f32).sqrt();
        let loud = rms >= LOUD_RMS;
        let start = frame.start;
        let end = start
            + Duration::from_micros(
                frame.samples.len() as u64 * 1_000_000 / u64::from(sv_audio::TARGET_SAMPLE_RATE),
            );
        let mut state = self.lock();
        match state.audio.back_mut() {
            Some((_, last_end, last_loud)) if *last_loud == loud && start <= *last_end => {
                *last_end = (*last_end).max(end);
            }
            _ => state.audio.push_back((start, end, loud)),
        }
        while state
            .audio
            .front()
            .is_some_and(|(_, e, _)| end.saturating_sub(*e) > AUDIO_HISTORY)
        {
            state.audio.pop_front();
        }
    }

    /// Fine dell'audio di sistema: niente più attese lunghe.
    pub fn close(&self) {
        self.lock().closed.get_or_insert_with(Instant::now);
        self.notify.notify_waiters();
    }

    /// Frase finale di "Loro" (tempi di cattura).
    pub fn them_final(&self, text: &str, start: Duration, end: Duration) {
        let words = words(text);
        let now = Instant::now();
        {
            let mut state = self.lock();
            if !words.is_empty() {
                state.recent.push_back(TheirFinal {
                    arrived: now,
                    start,
                    end,
                    words,
                });
            }
            prune(&mut state.recent, now);
        }
        self.notify.notify_waiters();
    }

    /// `true` se la frase di "Io" (`start`–`end`, tempi di cattura) è eco di
    /// "Loro". Se l'audio di sistema aveva voce nello stesso intervallo,
    /// attende fino a [`WAIT`] la frase di "Loro" corrispondente.
    pub async fn is_echo(&self, text: &str, start: Duration, end: Duration) -> bool {
        let mine = words(text);
        if mine.is_empty() {
            return false;
        }
        let mut deadline = Instant::now() + WAIT;
        loop {
            // Registrati prima del controllo: nessuna notifica persa.
            let notified = self.notify.notified();
            let them_loud = {
                let mut state = self.lock();
                if let Some(closed) = state.closed {
                    deadline = deadline.min(closed + WAIT_AFTER_CLOSE);
                }
                prune(&mut state.recent, Instant::now());
                if matches(&mine, start, end, &state.recent) {
                    return true;
                }
                // Audio di sistema non ancora arrivato fin lì: nel dubbio si
                // aspetta, come se avesse voce.
                let covered =
                    state.closed.is_some() || state.audio.back().is_some_and(|(_, e, _)| *e >= end);
                !covered || loud_share(&state.audio, start, end) >= LOUD_SHARE
            };
            let now = Instant::now();
            if !them_loud || now >= deadline {
                tracing::debug!(
                    text,
                    them_loud,
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

fn prune(recent: &mut VecDeque<TheirFinal>, now: Instant) {
    while recent
        .front()
        .is_some_and(|f| now.duration_since(f.arrived) > RECENT)
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

/// Quota di `[start, end)` in cui l'audio di sistema aveva voce.
fn loud_share(audio: &VecDeque<(Duration, Duration, bool)>, start: Duration, end: Duration) -> f32 {
    let span = end.saturating_sub(start);
    if span.is_zero() {
        return 0.0;
    }
    let loud: Duration = audio
        .iter()
        .filter(|(_, _, loud)| *loud)
        .map(|(s, e, _)| overlap(*s, *e, start, end))
        .sum();
    loud.as_secs_f32() / span.as_secs_f32()
}

fn overlap(a_start: Duration, a_end: Duration, b_start: Duration, b_end: Duration) -> Duration {
    a_end.min(b_end).saturating_sub(a_start.max(b_start))
}

/// Eco se le parole di `mine` compaiono in ordine nel testo recente di
/// "Loro" (sottosequenza comune più lunga ≥ [`MATCH_RATIO`]): l'eco può
/// perdere o storpiare qualche parola, ma non cambia l'ordine. Una frase
/// corta deve comparire tutta in una frase di "Loro" sovrapposta nel tempo.
fn matches(mine: &[String], start: Duration, end: Duration, theirs: &VecDeque<TheirFinal>) -> bool {
    if mine.len() < MIN_WORDS {
        let span = end.saturating_sub(start).as_secs_f32().max(0.001);
        return theirs.iter().any(|f| {
            overlap(f.start, f.end, start, end).as_secs_f32() / span >= SHORT_OVERLAP
                && lcs(mine, f.words.iter()) == mine.len()
        });
    }
    let common = lcs(mine, theirs.iter().flat_map(|f| &f.words));
    common as f32 >= MATCH_RATIO * mine.len() as f32
}

/// Lunghezza della sottosequenza comune più lunga.
fn lcs<'a>(mine: &[String], theirs: impl Iterator<Item = &'a String>) -> usize {
    let theirs: Vec<&String> = theirs.collect();
    if theirs.is_empty() {
        return 0;
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
    prev[theirs.len()]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn s(secs: f32) -> Duration {
        Duration::from_secs_f32(secs)
    }

    /// Audio di sistema da `from` a `to` (s), con voce o muto.
    fn audio(g: &EchoGuard, from: f32, to: f32, loud: bool) {
        let level = if loud { 0.1 } else { 0.0 };
        let mut t = from;
        while t < to {
            g.them_audio(&AudioFrame {
                channel: sv_audio::Channel::Them,
                start: s(t),
                samples: Arc::from(vec![level; 320]),
            });
            t += 0.02;
        }
    }

    fn guard_with(texts: &[(&str, f32, f32)]) -> EchoGuard {
        let guard = EchoGuard::default();
        for (t, a, b) in texts {
            guard.them_final(t, s(*a), s(*b));
        }
        guard
    }

    #[tokio::test]
    async fn exact_echo_is_dropped() {
        let text = "Thanks everyone. Let's review the action items from last week.";
        let g = guard_with(&[(text, 1.0, 5.0)]);
        assert!(g.is_echo(text, s(1.1), s(5.1)).await);
    }

    #[tokio::test]
    async fn partial_and_noisy_echo_is_dropped() {
        let g = guard_with(&[
            ("Thanks everyone.", 1.0, 2.0),
            ("Let's review the action items from last week.", 2.5, 5.0),
        ]);
        assert!(
            g.is_echo(
                "everyone let's review the actions items from last week",
                s(1.5),
                s(5.1)
            )
            .await
        );
        // Dalla call reale: una parola storpiata su dieci.
        let g = guard_with(&[(
            "Ha inviato da un altro algoritmo che abbiamo di invio.",
            39.3,
            44.0,
        )]);
        assert!(
            g.is_echo(
                "Ha inviato da un altro algoritmo che abbiamo di indio.",
                s(39.4),
                s(44.2)
            )
            .await
        );
    }

    #[tokio::test]
    async fn short_echo_needs_time_overlap() {
        let g = guard_with(&[("Vediamo, eh.", 10.7, 11.9), ("Ctrl C.", 71.8, 73.0)]);
        assert!(g.is_echo("Vediamo, eh.", s(10.8), s(12.0)).await);
        assert!(g.is_echo("Ctrl C", s(71.9), s(73.2)).await);
        audio(&g, 0.0, 90.0, false);
        // Stesse parole ma in un altro momento: è una risposta, non eco.
        assert!(!g.is_echo("Vediamo, eh.", s(80.0), s(81.0)).await);
    }

    #[tokio::test]
    async fn own_reply_is_kept() {
        let g = guard_with(&[("Do you agree with the plan?", 0.0, 2.0)]);
        audio(&g, 0.0, 10.0, false);
        assert!(
            !g.is_echo(
                "Yes, I agree with the plan, but not on timing.",
                s(3.0),
                s(6.0)
            )
            .await
        );
        assert!(!g.is_echo("Mi sentite?", s(7.0), s(8.0)).await);
    }

    #[tokio::test]
    async fn waits_for_late_final_of_them() {
        let g = Arc::new(EchoGuard::default());
        audio(&g, 0.0, 6.0, true);
        let late = g.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            late.them_final(
                "Let's review the action items from last week.",
                s(1.0),
                s(5.0),
            );
        });
        assert!(
            g.is_echo("review the action items from last week", s(1.1), s(5.1))
                .await
        );
    }

    #[tokio::test]
    async fn no_wait_when_system_audio_silent() {
        let g = EchoGuard::default();
        audio(&g, 0.0, 6.0, false);
        let t = Instant::now();
        assert!(
            !g.is_echo("review the action items from last week", s(1.0), s(5.0))
                .await
        );
        assert!(t.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn loud_share_counts_only_voice() {
        let g = EchoGuard::default();
        audio(&g, 0.0, 2.0, false);
        audio(&g, 2.0, 4.0, true);
        let state = g.lock();
        let share = loud_share(&state.audio, s(1.0), s(3.0));
        assert!((share - 0.5).abs() < 0.05, "{share}");
    }
}
