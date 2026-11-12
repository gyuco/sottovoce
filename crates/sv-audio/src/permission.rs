//! Permesso del microfono.
//!
//! macOS: `AVCaptureDevice` (TCC "Microfono"). Il permesso dell'audio di
//! sistema non ha un'API pubblica di verifica: senza permesso il tap
//! restituisce silenzio, quindi si verifica ascoltando il livello.

use serde::Serialize;

/// Stato del permesso del microfono.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MicPermission {
    Granted,
    Denied,
    /// Bloccato da un profilo di gestione (MDM, controlli parentali).
    Restricted,
    /// Mai chiesto: la prima cattura (o [`request_mic_permission`]) mostra il dialogo.
    Undetermined,
    /// La piattaforma non ha un permesso da chiedere.
    Unsupported,
}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::Mutex;

    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, Bool};
    use objc2_foundation::NSString;

    use super::MicPermission;

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        static AVMediaTypeAudio: &'static NSString;
    }

    fn device_class() -> Option<&'static AnyClass> {
        AnyClass::get(c"AVCaptureDevice")
    }

    pub fn status() -> MicPermission {
        let Some(cls) = device_class() else {
            return MicPermission::Unsupported;
        };
        // SAFETY: metodo di classe documentato; `AVMediaTypeAudio` è una costante del framework.
        let status: isize =
            unsafe { msg_send![cls, authorizationStatusForMediaType: AVMediaTypeAudio] };
        match status {
            0 => MicPermission::Undetermined,
            1 => MicPermission::Restricted,
            2 => MicPermission::Denied,
            3 => MicPermission::Granted,
            _ => MicPermission::Unsupported,
        }
    }

    pub fn request(done: Box<dyn FnOnce(bool) + Send>) {
        let Some(cls) = device_class() else {
            done(false);
            return;
        };
        // Il blocco è `Fn`: la callback va consumata una volta sola.
        let done = Mutex::new(Some(done));
        let block = RcBlock::new(move |granted: Bool| {
            if let Some(done) = done.lock().ok().and_then(|mut d| d.take()) {
                done(granted.as_bool());
            }
        });
        // SAFETY: il framework copia il blocco e lo chiama una volta, su una coda qualsiasi.
        unsafe {
            let _: () = msg_send![
                cls,
                requestAccessForMediaType: AVMediaTypeAudio,
                completionHandler: &*block
            ];
        }
    }
}

/// Stato attuale del permesso del microfono.
pub fn mic_permission() -> MicPermission {
    #[cfg(target_os = "macos")]
    {
        macos::status()
    }
    #[cfg(not(target_os = "macos"))]
    {
        MicPermission::Unsupported
    }
}

/// Chiede il permesso del microfono (dialogo di sistema se mai chiesto);
/// `done` riceve l'esito, da un thread qualsiasi.
pub fn request_mic_permission(done: impl FnOnce(bool) + Send + 'static) {
    #[cfg(target_os = "macos")]
    {
        macos::request(Box::new(done));
    }
    #[cfg(not(target_os = "macos"))]
    {
        done(true);
    }
}
