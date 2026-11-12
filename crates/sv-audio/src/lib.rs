//! Cattura audio per Sottovoce.
//!
//! Due canali indipendenti:
//! - [`Channel::Me`]: microfono (via `cpal`).
//! - [`Channel::Them`]: audio di sistema (macOS: Core Audio process tap).
//!
//! Ogni sorgente scrive campioni grezzi in un ring buffer lock-free dal thread
//! realtime; un thread worker per canale fa downmix, calcola i livelli e
//! ricampiona a 16 kHz mono, pubblicando [`AudioFrame`] su un canale broadcast.
//!
//! [`EchoCanceller`] toglie dal microfono l'eco dell'audio di sistema.

mod aec;
mod capture;
mod error;
mod frame;
mod level;
pub mod mic;
mod permission;
mod resample;
mod source;
pub mod system;

pub use aec::EchoCanceller;
pub use capture::{Capture, CaptureConfig};
pub use error::AudioError;
pub use frame::{AudioFrame, Channel, LevelEvent, TARGET_SAMPLE_RATE};
pub use mic::{InputDevice, list_input_devices};
pub use permission::{MicPermission, mic_permission, request_mic_permission};
