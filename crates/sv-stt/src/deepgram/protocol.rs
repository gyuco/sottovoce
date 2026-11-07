//! Messaggi del WebSocket Deepgram `/v1/listen` e accumulo in frasi.
//!
//! Deepgram manda per ogni porzione d'audio:
//! - `is_final=false`: ipotesi provvisoria (può cambiare);
//! - `is_final=true`: testo definitivo della porzione, ma la frase può continuare;
//! - `speech_final=true` (o un `UtteranceEnd`): fine della frase.
//!
//! [`Utterance`] unisce le porzioni definitive fino alla fine frase, così a
//! valle (UI, traduzione) arrivano frasi intere e non spezzoni.

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
}

impl Utterance {
    /// Applica un `Results`. `origin` = tempo di cattura del primo frame
    /// inviato su questa connessione (i tempi Deepgram sono relativi a esso).
    pub fn on_results(&mut self, r: &Results, origin: Duration) -> Option<SttEventKind> {
        let alt = r.channel.alternatives.first()?;
        let piece = alt.transcript.trim();
        let start = origin + secs(r.start);
        let end = origin + secs(r.start + r.duration);

        if !r.is_final {
            if piece.is_empty() {
                return None;
            }
            let seg = Segment {
                text: join(&self.text, piece),
                start: self.start.unwrap_or(start),
                end,
                confidence: alt.confidence,
            };
            return Some(SttEventKind::Partial(seg));
        }

        if !piece.is_empty() {
            self.text = join(&self.text, piece);
            self.start.get_or_insert(start);
            self.end = end;
            self.confidence = min_conf(self.confidence, alt.confidence);
        }
        if r.speech_final {
            return self.flush();
        }
        // Porzione definitiva a metà frase: la mostriamo come parziale.
        (!piece.is_empty()).then(|| SttEventKind::Partial(self.segment()))
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
        }
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
        let p = u.on_results(&results(0.0, 1.0, "ciao a", false, false), O);
        assert!(matches!(p, Some(SttEventKind::Partial(s)) if s.text == "ciao a"));

        let p = u.on_results(&results(0.0, 1.5, "ciao a tutti", true, false), O);
        assert!(matches!(&p, Some(SttEventKind::Partial(s)) if s.text == "ciao a tutti"));

        let p = u.on_results(&results(1.5, 0.5, "come", false, false), O);
        assert!(matches!(&p, Some(SttEventKind::Partial(s))
            if s.text == "ciao a tutti come" && s.start == O));

        let f = u.on_results(&results(1.5, 1.0, "come state?", true, true), O);
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
        assert_eq!(u.on_results(&results(0.0, 1.0, "", false, false), O), None);
        assert_eq!(u.on_results(&results(0.0, 1.0, "", true, true), O), None);
        u.on_results(&results(1.0, 1.0, "hello", true, false), O);
        assert!(matches!(u.flush(), Some(SttEventKind::Final(s)) if s.text == "hello"));
        assert_eq!(u.flush(), None);
    }
}
