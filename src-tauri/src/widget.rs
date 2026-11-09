//! Widget overlay: finestra piccola sempre in primo piano sopra Teams, Meet o
//! Zoom, con le ultime frasi tradotte, i suggerimenti e i nudge.
//!
//! I dati non passano da qui: la finestra principale invia al widget uno
//! snapshot con gli eventi Tauri (vedi `src/lib/widget.ts`). Posizione e
//! dimensione le ricorda `tauri-plugin-window-state`.

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LABEL: &str = "widget";

/// Scorciatoia globale che mostra o nasconde il widget.
pub const SHORTCUT: &str = "Alt+Super+S";

/// Mostra il widget se chiuso, lo chiude se aperto.
pub fn toggle<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<bool> {
    if let Some(window) = app.get_webview_window(LABEL) {
        window.close()?;
        return Ok(false);
    }
    open(app)?;
    Ok(true)
}

/// Apre il widget (o lo riporta in primo piano se già aperto).
pub fn open<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(LABEL) {
        return window.show();
    }
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Sottovoce")
        .inner_size(380.0, 460.0)
        .min_inner_size(280.0, 200.0)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .resizable(true)
        // Escluso dalla condivisione schermo (macOS: `sharingType = none`).
        .content_protected(true)
        // Non ruba il focus alla call quando compare.
        .focused(false)
        .build()?;
    float_over_fullscreen(&window);
    Ok(())
}

/// Chiude il widget se aperto (es. quando si chiude la finestra principale).
pub fn close<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(LABEL)
        && let Err(e) = window.close()
    {
        tracing::warn!("chiusura del widget non riuscita: {e}");
    }
}

/// macOS: `always_on_top` basta sopra le finestre normali, non sopra una call
/// a schermo intero (Space dedicato). Serve `FullScreenAuxiliary` e un livello
/// sopra quello delle finestre flottanti.
#[cfg(target_os = "macos")]
fn float_over_fullscreen<R: Runtime>(window: &WebviewWindow<R>) {
    let Ok(ptr) = window.ns_window() else {
        tracing::warn!("NSWindow del widget non disponibile");
        return;
    };
    let ptr = ptr as usize;
    let result = window.run_on_main_thread(move || {
        use objc2_app_kit::{NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior};
        // SAFETY: puntatore valido dell'NSWindow del widget, usato sul main
        // thread mentre la finestra esiste (appena creata).
        let ns_window = unsafe { &*(ptr as *const NSWindow) };
        ns_window.setCollectionBehavior(
            ns_window.collectionBehavior()
                | NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        ns_window.setLevel(NSStatusWindowLevel);
    });
    if let Err(e) = result {
        tracing::warn!("widget sopra lo schermo intero non impostato: {e}");
    }
}

#[cfg(not(target_os = "macos"))]
fn float_over_fullscreen<R: Runtime>(_window: &WebviewWindow<R>) {}
