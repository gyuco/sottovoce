//! Motore finto e deterministico per test e sviluppo UI.

use std::time::Duration;

use sv_audio::{AudioFrame, Channel};
use tokio::sync::mpsc;

use crate::engine::{EventSender, Language, SttEngine, SttStream};
use crate::event::{Segment, SttEventKind};

/// Per ogni frame ricevuto emette un `Partial` con una parola in più
/// ("w1 w2 …"); ogni `frames_per_final` frame emette un `Final` e riparte.
/// A fine audio chiude l'eventuale frase aperta con un `Final`.
#[derive(Debug, Clone)]
pub struct FakeEngine {
    pub frames_per_final: usize,
}

impl Default for FakeEngine {
    fn default() -> Self {
        Self {
            frames_per_final: 4,
        }
    }
}

impl SttEngine for FakeEngine {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn start(&self, channel: Channel, _language: Language) -> SttStream {
        let per_final = self.frames_per_final.max(1);
        SttStream::spawn(channel, move |audio, events| run(audio, events, per_final))
    }
}

async fn run(mut audio: mpsc::Receiver<AudioFrame>, events: EventSender, per_final: usize) {
    if !events.send(SttEventKind::Connected).await {
        return;
    }
    let mut words = 0usize;
    let mut start = Duration::ZERO;
    let mut end = Duration::ZERO;
    while let Some(frame) = audio.recv().await {
        if words == 0 {
            start = frame.start;
        }
        end = frame.start + Duration::from_secs_f64(frame.samples.len() as f64 / 16_000.0);
        words += 1;
        let seg = segment(words, start, end);
        let kind = if words == per_final {
            words = 0;
            SttEventKind::Final(seg)
        } else {
            SttEventKind::Partial(seg)
        };
        if !events.send(kind).await {
            return;
        }
    }
    if words > 0 {
        events
            .send(SttEventKind::Final(segment(words, start, end)))
            .await;
    }
    events.send(SttEventKind::Disconnected).await;
}

fn segment(words: usize, start: Duration, end: Duration) -> Segment {
    let text = (1..=words)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    Segment {
        text,
        start,
        end,
        confidence: Some(1.0),
        ..Segment::default()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::SttEvent;

    fn frame(i: u64) -> AudioFrame {
        AudioFrame {
            channel: Channel::Me,
            start: Duration::from_millis(i * 100),
            samples: Arc::from(vec![0.0f32; 1600]),
        }
    }

    async fn collect(n_frames: u64, per_final: usize) -> Vec<SttEvent> {
        let engine = FakeEngine {
            frames_per_final: per_final,
        };
        let mut stream = engine.start(Channel::Me, Language::Multi);
        for i in 0..n_frames {
            stream.audio.send(frame(i)).await.unwrap();
        }
        drop(stream.audio);
        let mut out = Vec::new();
        while let Some(e) = stream.events.recv().await {
            out.push(e);
        }
        stream.task.await.unwrap();
        out
    }

    #[tokio::test]
    async fn partial_then_final_then_disconnect() {
        let ev = collect(3, 2).await;
        let kinds: Vec<_> = ev.iter().map(|e| &e.kind).collect();
        assert_eq!(kinds[0], &SttEventKind::Connected);
        assert!(matches!(kinds[1], SttEventKind::Partial(s) if s.text == "w1"));
        assert!(
            matches!(kinds[2], SttEventKind::Final(s) if s.text == "w1 w2"
            && s.start == Duration::ZERO && s.end == Duration::from_millis(200))
        );
        assert!(matches!(kinds[3], SttEventKind::Partial(s) if s.text == "w1"));
        // Fine audio: la frase aperta viene chiusa.
        assert!(matches!(kinds[4], SttEventKind::Final(s) if s.text == "w1"
            && s.start == Duration::from_millis(200)));
        assert_eq!(kinds[5], &SttEventKind::Disconnected);
        assert_eq!(ev.len(), 6);
        assert!(ev.iter().all(|e| e.channel == Channel::Me));
    }

    #[tokio::test]
    async fn no_audio_just_connect_and_disconnect() {
        let ev = collect(0, 4).await;
        let kinds: Vec<_> = ev.into_iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![SttEventKind::Connected, SttEventKind::Disconnected]
        );
    }

    #[test]
    fn event_json_shape() {
        let e = SttEvent {
            channel: Channel::Them,
            kind: SttEventKind::Final(Segment {
                text: "ciao".into(),
                start: Duration::from_millis(1500),
                end: Duration::from_millis(2250),
                id: 7,
                language: Some("en".into()),
                ..Segment::default()
            }),
        };
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "channel": "them",
                "type": "final",
                "text": "ciao",
                "start": 1500,
                "end": 2250,
                "confidence": null,
                "id": 7,
                "language": "en"
            })
        );
    }
}
