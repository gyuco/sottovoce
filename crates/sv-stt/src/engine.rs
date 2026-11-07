use serde::{Deserialize, Serialize};
use sv_audio::{AudioFrame, Channel};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::event::SttEvent;

/// Frame audio in coda verso il motore (~64 ms l'uno → ~16 s).
pub const AUDIO_QUEUE: usize = 256;
/// Eventi in coda verso il consumatore.
pub const EVENT_QUEUE: usize = 256;

/// Lingua del parlato.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// Rilevamento automatico / code-switching.
    #[default]
    Multi,
    It,
    En,
}

impl Language {
    /// Lingue concrete note all'app (usate come hint in modalità `Multi`).
    pub const CONCRETE: [Language; 2] = [Language::It, Language::En];

    pub fn code(self) -> &'static str {
        match self {
            Self::Multi => "multi",
            Self::It => "it",
            Self::En => "en",
        }
    }
}

/// Motore speech-to-text. Implementazioni: cloud (WebSocket) e locali.
pub trait SttEngine: Send + Sync {
    /// Nome del provider (log, errori).
    fn name(&self) -> &'static str;

    /// Apre uno stream per un canale. Va chiamato dentro un runtime tokio:
    /// la connessione avviene in un task in background e i suoi esiti
    /// (inclusi gli errori) arrivano come [`SttEvent`].
    fn start(&self, channel: Channel, language: Language) -> SttStream;
}

/// Stream STT di un canale.
///
/// Chiudere `audio` (drop del sender) segnala fine audio: il motore svuota,
/// emette gli ultimi `Final` e poi `Disconnected`, quindi chiude `events`.
pub struct SttStream {
    pub channel: Channel,
    pub audio: mpsc::Sender<AudioFrame>,
    pub events: mpsc::Receiver<SttEvent>,
    pub task: JoinHandle<()>,
}

impl SttStream {
    /// Crea le code e avvia `run` come task tokio. Helper per i motori.
    pub fn spawn<F, Fut>(channel: Channel, run: F) -> Self
    where
        F: FnOnce(mpsc::Receiver<AudioFrame>, EventSender) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (audio_tx, audio_rx) = mpsc::channel(AUDIO_QUEUE);
        let (event_tx, event_rx) = mpsc::channel(EVENT_QUEUE);
        let task = tokio::spawn(run(
            audio_rx,
            EventSender {
                channel,
                tx: event_tx,
            },
        ));
        Self {
            channel,
            audio: audio_tx,
            events: event_rx,
            task,
        }
    }
}

/// Lato motore della coda eventi: aggiunge il canale a ogni evento.
#[derive(Clone)]
pub struct EventSender {
    channel: Channel,
    tx: mpsc::Sender<SttEvent>,
}

impl EventSender {
    /// Ritorna false se il consumatore ha chiuso la coda.
    pub async fn send(&self, kind: crate::SttEventKind) -> bool {
        self.tx
            .send(SttEvent {
                channel: self.channel,
                kind,
            })
            .await
            .is_ok()
    }
}
