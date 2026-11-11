//! Messaggi del WebSocket Deepgram `/v1/listen` e accumulo in frasi.
//!
//! Deepgram manda per ogni porzione d'audio:
//! - `is_final=false`: ipotesi provvisoria (può cambiare);
//! - `is_final=true`: testo definitivo della porzione, ma la frase può continuare;
//! - `speech_final=true` (o un `UtteranceEnd`): fine della frase.
//!
//! [`Utterance`] unisce le porzioni definitive fino alla fine frase, così a
//! valle (UI, traduzione) arrivano frasi intere e non spezzoni.
//!
//! Con `diarize=true` ogni parola ha il suo parlante: la frase si chiude
//! anche quando il parlante cambia, così due voci non finiscono nella stessa
//! frase, e il `Final` porta il parlante di Deepgram.

use std::time::Duration;

use serde::Deserialize;

use crate::event::{Segment, SttEventKind};

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    Results(Results),
    UtteranceEnd {},
    SpeechStarted {},
    Metadata {},
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub struct Results {
    /// Secondi dall'inizio dell'audio inviato su questa connessione.
    pub start: f64,
    pub duration: f64,
    #[serde(default)]
    pub is_final: bool,
    #[serde(default)]
    pub speech_final: bool,
    pub channel: ResultChannel,
}

#[derive(Debug, Deserialize)]
pub struct ResultChannel {
    pub alternatives: Vec<Alternative>,
}

#[derive(Debug, Deserialize)]
pub struct Alternative {
    pub transcript: String,
    #[serde(default)]
    pub confidence: Option<f32>,
    /// Lingue rilevate (solo con `language=multi`), la prevalente per prima.
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub words: Vec<Word>,
}

#[derive(Debug, Deserialize)]
pub struct Word {
    #[serde(default)]
    pub word: String,
    /// Con `smart_format`/`punctuate`: parola con maiuscole e punteggiatura.
    #[serde(default)]
    pub punctuated_word: Option<String>,
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub end: f64,
    /// Solo con `diarize=true`.
    #[serde(default)]
    pub speaker: Option<u32>,
}

/// Tratto di una porzione detto da un solo parlante.
struct Run {
    speaker: Option<u8>,
    text: String,
    words: usize,
    start: f64,
    end: f64,
}

/// Divide la porzione per parlante. Senza parlanti (diarization spenta) è
/// un solo tratto con i tempi della porzione. Un cambio di una sola parola
/// in mezzo a un altro parlante è quasi sempre un errore: resta al tratto
/// precedente.
fn runs(r: &Results, alt: &Alternative, piece: &str) -> Vec<Run> {
    let whole = || Run {
        speaker: None,
        text: piece.to_owned(),
        words: piece.split_whitespace().count(),
        start: r.start,
        end: r.start + r.duration,
    };
    if alt.words.is_empty() || alt.words.iter().all(|w| w.speaker.is_none()) {
        return vec![whole()];
    }
    let mut out: Vec<Run> = Vec::new();
    for (i, w) in alt.words.iter().enumerate() {
        let text = w.punctuated_word.as_deref().unwrap_or(&w.word);
        let speaker = w.speaker.and_then(|s| u8::try_from(s).ok());
        let next = alt.words.get(i + 1).map(|n| n.speaker);
        let isolated = out.last().is_some_and(|l| l.speaker != speaker)
            && next.is_some_and(|n| n.and_then(|s| u8::try_from(s).ok()) != speaker);
        match out.last_mut() {
            Some(last) if last.speaker == speaker || isolated => {
                last.text = join(&last.text, text);
                last.words += 1;
                last.end = w.end;
            }
            _ => out.push(Run {
                speaker,
                text: text.to_owned(),
                words: 1,
                start: w.start,
                end: w.end,
            }),
        }
    }
    out
}

pub fn parse(text: &str) -> Result<ServerMessage, serde_json::Error> {
    serde_json::from_str(text)
}

/// Frase in costruzione su una connessione.
#[derive(Debug, Default)]
pub struct Utterance {
    text: String,
    start: Option<Duration>,
    end: Duration,
    confidence: Option<f32>,
    /// Parole per lingua rilevata nelle porzioni definitive.
    language_words: Vec<(String, usize)>,
    /// Parlante (diarization di Deepgram), se attiva.
    speaker: Option<u8>,
}

impl Utterance {
    /// Applica un `Results`. `origin` = tempo di cattura del primo frame
    /// inviato su questa connessione (i tempi Deepgram sono relativi a esso).
    /// Più eventi se il parlante cambia nella porzione.
    pub fn on_results(&mut self, r: &Results, origin: Duration) -> Vec<SttEventKind> {
        let Some(alt) = r.channel.alternatives.first() else {
            return Vec::new();
        };
        let piece = alt.transcript.trim();
        let start = origin + secs(r.start);
        let end = origin + secs(r.start + r.duration);

        if !r.is_final {
            if piece.is_empty() {
                return Vec::new();
            }
            let seg = Segment {
                text: join(&self.text, piece),
                start: self.start.unwrap_or(start),
                end,
                confidence: alt.confidence,
                language: self.language().or_else(|| alt.languages.first().cloned()),
                speaker: None,
                ..Segment::default()
            };
            return vec![SttEventKind::Partial(seg)];
        }

        let mut out = Vec::new();
        if !piece.is_empty() {
            for run in runs(r, alt, piece) {
                // Cambio di parlante: la frase precedente si chiude qui.
                if !self.text.is_empty()
                    && run.speaker.is_some()
                    && self.speaker.is_some()
                    && run.speaker != self.speaker
                {
                    out.extend(self.flush());
                }
                self.text = join(&self.text, &run.text);
                self.start.get_or_insert(origin + secs(run.start));
                self.end = origin + secs(run.end);
                self.speaker = run.speaker.or(self.speaker);
                self.confidence = min_conf(self.confidence, alt.confidence);
                if let Some(lang) = alt.languages.first() {
                    match self.language_words.iter_mut().find(|(l, _)| l == lang) {
                        Some((_, n)) => *n += run.words,
                        None => self.language_words.push((lang.clone(), run.words)),
                    }
                }
            }
        }
        if r.speech_final {
            out.extend(self.flush());
        } else if !piece.is_empty() {
            // Porzione definitiva a metà frase: la mostriamo come parziale.
            out.push(SttEventKind::Partial(self.segment()));
        }
        out
    }

    /// Chiude la frase corrente, se c'è (UtteranceEnd, fine stream, riconnessione).
    pub fn flush(&mut self) -> Option<SttEventKind> {
        if self.text.is_empty() {
            *self = Self::default();
            return None;
        }
        let seg = self.segment();
        *self = Self::default();
        Some(SttEventKind::Final(seg))
    }

    fn segment(&self) -> Segment {
        Segment {
            text: self.text.clone(),
            start: self.start.unwrap_or(self.end),
            end: self.end,
            confidence: self.confidence,
            language: self.language(),
            speaker: self.speaker,
            ..Segment::default()
        }
    }

    /// Lingua con più parole nella frase (a parità, la prima incontrata).
    fn language(&self) -> Option<String> {
        self.language_words
            .iter()
            .fold(None::<&(String, usize)>, |best, cur| match best {
                Some(b) if b.1 >= cur.1 => Some(b),
                _ => Some(cur),
            })
            .map(|(l, _)| l.clone())
    }
}

fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s.max(0.0))
}

fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.to_owned()
    } else {
        format!("{a} {b}")
    }
}

fn min_conf(a: Option<f32>, b: Option<f32>) -> Option<f32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Forma reale di un messaggio `Results` di Deepgram (campi extra ignorati).
    fn results(start: f64, dur: f64, text: &str, is_final: bool, speech_final: bool) -> Results {
        let json = format!(
            r#"{{"type":"Results","channel_index":[0,1],"duration":{dur},"start":{start},
            "is_final":{is_final},"speech_final":{speech_final},"from_finalize":false,
            "channel":{{"alternatives":[{{"transcript":"{text}","confidence":0.9,
            "words":[],"languages":["it"]}}]}},
            "metadata":{{"request_id":"x","model_info":{{"name":"general-nova-3"}}}}}}"#
        );
        match parse(&json).unwrap() {
            ServerMessage::Results(r) => r,
            other => panic!("atteso Results, trovato {other:?}"),
        }
    }

    const O: Duration = Duration::from_secs(10);

    /// Al più un evento (porzioni senza cambio di parlante).
    fn one(u: &mut Utterance, r: &Results) -> Option<SttEventKind> {
        let mut out = u.on_results(r, O);
        assert!(out.len() <= 1, "{out:?}");
        out.pop()
    }

    /// Porzione definitiva con parole e parlanti (`diarize=true`).
    fn diarized(words: &[(&str, f64, u32)], speech_final: bool) -> Results {
        let list: Vec<String> = words
            .iter()
            .map(|(w, t, s)| {
                format!(
                    r#"{{"word":"{w}","punctuated_word":"{w}","start":{t},"end":{},"speaker":{s}}}"#,
                    t + 0.3
                )
            })
            .collect();
        let text: Vec<&str> = words.iter().map(|(w, _, _)| *w).collect();
        let start = words.first().map_or(0.0, |w| w.1);
        let json = format!(
            r#"{{"type":"Results","duration":3.0,"start":{start},"is_final":true,
            "speech_final":{speech_final},"channel":{{"alternatives":[{{"transcript":"{}",
            "words":[{}]}}]}}}}"#,
            text.join(" "),
            list.join(",")
        );
        match parse(&json).unwrap() {
            ServerMessage::Results(r) => r,
            other => panic!("atteso Results, trovato {other:?}"),
        }
    }

    fn finals(events: &[SttEventKind]) -> Vec<(&str, Option<u8>)> {
        events
            .iter()
            .filter_map(|e| match e {
                SttEventKind::Final(s) => Some((s.text.as_str(), s.speaker)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn speaker_change_splits_the_sentence() {
        let mut u = Utterance::default();
        let out = u.on_results(
            &diarized(
                &[
                    ("Hola", 0.0, 0),
                    ("Raúl.", 0.4, 0),
                    ("Buen", 1.0, 1),
                    ("día.", 1.3, 1),
                ],
                false,
            ),
            O,
        );
        assert_eq!(finals(&out), vec![("Hola Raúl.", Some(0))]);
        let out = u.on_results(&diarized(&[("¿Cómo", 2.0, 1), ("estás?", 2.3, 1)], true), O);
        assert_eq!(finals(&out), vec![("Buen día. ¿Cómo estás?", Some(1))]);
    }

    #[test]
    fn isolated_word_does_not_split() {
        let mut u = Utterance::default();
        let out = u.on_results(
            &diarized(&[("uno", 0.0, 0), ("dos", 0.4, 1), ("tres", 0.8, 0)], true),
            O,
        );
        assert_eq!(finals(&out), vec![("uno dos tres", Some(0))]);
    }

    #[test]
    fn without_diarization_speaker_is_none() {
        let mut u = Utterance::default();
        let f = one(&mut u, &results(0.0, 1.0, "ciao", true, true));
        assert!(matches!(f, Some(SttEventKind::Final(s)) if s.speaker.is_none()));
    }

    fn results_lang(text: &str, langs: &str, speech_final: bool) -> Results {
        let json = format!(
            r#"{{"type":"Results","duration":1.0,"start":0.0,"is_final":true,
            "speech_final":{speech_final},"channel":{{"alternatives":[{{"transcript":"{text}",
            "languages":{langs}}}]}}}}"#
        );
        match parse(&json).unwrap() {
            ServerMessage::Results(r) => r,
            other => panic!("atteso Results, trovato {other:?}"),
        }
    }

    #[test]
    fn final_language_is_the_dominant_one() {
        let mut u = Utterance::default();
        one(&mut u, &results_lang("ok allora", r#"["it"]"#, false));
        one(
            &mut u,
            &results_lang("we should ship it next week", r#"["en","it"]"#, false),
        );
        let f = one(&mut u, &results_lang("", "[]", true));
        let Some(SttEventKind::Final(s)) = f else {
            panic!("atteso Final")
        };
        assert_eq!(s.language.as_deref(), Some("en"));
        assert_eq!(s.id, 0);
    }

    #[test]
    fn no_language_without_detection() {
        let mut u = Utterance::default();
        let f = one(&mut u, &results_lang("ciao", "[]", true));
        assert!(matches!(f, Some(SttEventKind::Final(s)) if s.language.is_none()));
    }

    #[test]
    fn parses_other_message_types() {
        assert!(matches!(
            parse(r#"{"type":"UtteranceEnd","channel":[0,1],"last_word_end":2.4}"#).unwrap(),
            ServerMessage::UtteranceEnd {}
        ));
        assert!(matches!(
            parse(r#"{"type":"Metadata","request_id":"abc","duration":3.1}"#).unwrap(),
            ServerMessage::Metadata {}
        ));
        assert!(matches!(
            parse(r#"{"type":"Nuovo"}"#).unwrap(),
            ServerMessage::Other
        ));
    }

    #[test]
    fn interim_pieces_and_speech_final_build_one_sentence() {
        let mut u = Utterance::default();
        let p = one(&mut u, &results(0.0, 1.0, "ciao a", false, false));
        assert!(matches!(p, Some(SttEventKind::Partial(s)) if s.text == "ciao a"));

        let p = one(&mut u, &results(0.0, 1.5, "ciao a tutti", true, false));
        assert!(matches!(&p, Some(SttEventKind::Partial(s)) if s.text == "ciao a tutti"));

        let p = one(&mut u, &results(1.5, 0.5, "come", false, false));
        assert!(matches!(&p, Some(SttEventKind::Partial(s))
            if s.text == "ciao a tutti come" && s.start == O));

        let f = one(&mut u, &results(1.5, 1.0, "come state?", true, true));
        let Some(SttEventKind::Final(s)) = f else {
            panic!("atteso Final")
        };
        assert_eq!(s.text, "ciao a tutti come state?");
        assert_eq!(s.start, O);
        assert_eq!(s.end, O + Duration::from_millis(2500));
        assert_eq!(u.flush(), None);
    }

    #[test]
    fn empty_results_are_ignored_and_utterance_end_flushes() {
        let mut u = Utterance::default();
        assert_eq!(one(&mut u, &results(0.0, 1.0, "", false, false)), None);
        assert_eq!(one(&mut u, &results(0.0, 1.0, "", true, true)), None);
        one(&mut u, &results(1.0, 1.0, "hello", true, false));
        assert!(matches!(u.flush(), Some(SttEventKind::Final(s)) if s.text == "hello"));
        assert_eq!(u.flush(), None);
    }
}
