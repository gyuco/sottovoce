//! Speech-to-text per Sottovoce.
//!
//! Il trait [`SttEngine`] astrae il motore (cloud o locale). Ogni canale audio
//! ("Io" / "Loro") apre il proprio [`SttStream`]: si inviano [`AudioFrame`]
//! 16 kHz mono su `audio`, si ricevono [`SttEvent`] su `events`.
//!
//! [`AudioFrame`]: sv_audio::AudioFrame

pub mod deepgram;
mod engine;
mod error;
mod event;
pub mod fake;

pub use engine::{AUDIO_QUEUE, EVENT_QUEUE, EventSender, Language, SttEngine, SttStream};
pub use error::SttError;
pub use event::{Segment, SpeakerUpdate, SttEvent, SttEventKind};
