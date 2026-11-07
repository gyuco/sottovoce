//! Audio di sistema ("Loro").

#[cfg(target_os = "macos")]
mod macos_tap;

use crate::error::AudioError;
use crate::source::OpenedSource;

/// Apre la cattura dell'audio di sistema. Va chiamata nel thread che la possiede.
pub(crate) fn open() -> Result<OpenedSource, AudioError> {
    #[cfg(target_os = "macos")]
    {
        macos_tap::open()
    }
    // TODO(windows): WASAPI loopback via cpal sul dispositivo di output di default.
    #[cfg(not(target_os = "macos"))]
    {
        Err(AudioError::SystemCaptureUnsupported)
    }
}

/// `true` se la piattaforma supporta la cattura dell'audio di sistema.
pub fn is_supported() -> bool {
    cfg!(target_os = "macos")
}
