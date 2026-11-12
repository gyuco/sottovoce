//! Sincronizzazione con Google Calendar: le riunioni con un link di call
//! (Meet, Teams, Zoom) diventano riunioni programmate nell'Agenda.
//!
//! - Collegamento: OAuth desktop (browser di sistema + redirect su loopback +
//!   PKCE, vedi `sv_calendar::oauth`). Il client OAuth lo crea l'utente su
//!   Google Cloud: ID client nei settings (`googleClientId`), client secret e
//!   refresh token nel Keychain. Nessun server intermedio.
//! - Sincronizzazione: all'avvio, ogni 10 minuti, dopo il collegamento e a
//!   richiesta. Finestra da 1 giorno fa a 30 giorni avanti; esclusi eventi
//!   senza link di call e inviti rifiutati. Evento `scheduled:changed` alla UI.
//! - Scollegamento: revoca del token presso Google, token cancellato, riunioni
//!   importate rimosse (tranne quelle modificate dall'utente).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use sv_calendar::{AccessToken, CalendarError, CallbackPage, OAuthClient};
use sv_store::{ExternalMeeting, ScheduledParticipant, Store, SyncReport};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::secrets;
use crate::settings::SettingsStore;
use crate::state::Db;

/// Valore di `scheduled_meetings.source`.
pub const SOURCE: &str = "google";
/// Account Keychain del client secret OAuth (vedi `secrets::KNOWN_PROVIDERS`).
pub const CLIENT_SECRET_KEY: &str = "google-client-secret";
/// Account Keychain del refresh token.
pub const TOKEN_KEY: &str = "google-calendar";
/// Evento alla UI: l'Agenda va ricaricata.
pub const CHANGED_EVENT: &str = "scheduled:changed";

const PAST_MS: i64 = 24 * 60 * 60 * 1000;
const FUTURE_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const SYNC_EVERY: Duration = Duration::from_secs(10 * 60);
const FIRST_SYNC_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarStatus {
    /// ID client e client secret presenti.
    pub configured: bool,
    /// Refresh token presente.
    pub connected: bool,
    /// Account collegato.
    pub email: String,
    pub connecting: bool,
    pub syncing: bool,
    pub last_sync_ms: Option<i64>,
    pub last_report: Option<SyncReport>,
    /// Ultimo errore di sincronizzazione o collegamento.
    pub last_error: Option<String>,
    /// L'accesso è stato revocato o è scaduto: va ricollegato.
    pub needs_reconnect: bool,
}

#[derive(Default)]
struct Runtime {
    connecting: Option<Arc<Notify>>,
    syncing: bool,
    last_sync_ms: Option<i64>,
    last_report: Option<SyncReport>,
    last_error: Option<String>,
    needs_reconnect: bool,
    access: Option<AccessToken>,
}

pub struct Calendar {
    http: reqwest::Client,
    runtime: Mutex<Runtime>,
    /// Una sincronizzazione alla volta.
    sync_lock: tokio::sync::Mutex<()>,
}

impl Calendar {
    pub fn new() -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("Sottovoce")
            .build()
            .context("client HTTP del calendario")?;
        Ok(Self {
            http,
            runtime: Mutex::default(),
            sync_lock: tokio::sync::Mutex::new(()),
        })
    }

    fn runtime(&self) -> std::sync::MutexGuard<'_, Runtime> {
        self.runtime.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn status(&self, settings: &SettingsStore) -> CalendarStatus {
        let settings = settings.get();
        let configured = !settings.google_client_id.trim().is_empty()
            && secrets::has_api_key(CLIENT_SECRET_KEY).unwrap_or(false);
        let connected = secrets::has_api_key(TOKEN_KEY).unwrap_or(false);
        let rt = self.runtime();
        CalendarStatus {
            configured,
            connected,
            email: settings.google_calendar_email,
            connecting: rt.connecting.is_some(),
            syncing: rt.syncing,
            last_sync_ms: rt.last_sync_ms,
            last_report: rt.last_report,
            last_error: rt.last_error.clone(),
            needs_reconnect: rt.needs_reconnect,
        }
    }

    fn client(settings: &SettingsStore) -> Result<OAuthClient> {
        let client_id = settings.get().google_client_id.trim().to_owned();
        if client_id.is_empty() {
            bail!("ID client di Google mancante");
        }
        let client_secret = secrets::get_api_key(CLIENT_SECRET_KEY)?
            .ok_or_else(|| anyhow!("client secret di Google mancante"))?;
        Ok(OAuthClient {
            client_id,
            client_secret,
        })
    }

    /// Apre il consenso nel browser e aspetta il ritorno (max 5 minuti o
    /// [`Calendar::cancel_connect`]); poi sincronizza.
    pub async fn connect(&self, app: &AppHandle, ui_language: &str) -> Result<()> {
        let settings = app.state::<SettingsStore>();
        let client = Self::client(&settings)?;
        let cancel = Arc::new(Notify::new());
        {
            let mut rt = self.runtime();
            if rt.connecting.is_some() {
                bail!("collegamento già in corso");
            }
            rt.connecting = Some(cancel.clone());
        }
        let result = sv_calendar::oauth::authorize(
            &self.http,
            &client,
            &callback_page(ui_language),
            open_in_browser,
            cancel.notified(),
            CONNECT_TIMEOUT,
        )
        .await;
        self.runtime().connecting = None;
        let authorized = match result {
            Ok(a) => a,
            Err(CalendarError::Cancelled) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        secrets::set_api_key(TOKEN_KEY, &authorized.refresh_token)?;
        settings.update(serde_json::json!({
            "googleCalendarEmail": authorized.email.unwrap_or_default()
        }))?;
        {
            let mut rt = self.runtime();
            rt.access = Some(authorized.access);
            rt.needs_reconnect = false;
            rt.last_error = None;
        }
        self.sync(app).await?;
        Ok(())
    }

    pub fn cancel_connect(&self) {
        if let Some(cancel) = &self.runtime().connecting {
            cancel.notify_one();
        }
    }

    /// Revoca l'accesso (se possibile), cancella il token e le riunioni
    /// importate non modificate.
    pub async fn disconnect(&self, app: &AppHandle) -> Result<()> {
        let _guard = self.sync_lock.lock().await;
        if let Ok(Some(token)) = secrets::get_api_key(TOKEN_KEY)
            && let Err(e) = sv_calendar::oauth::revoke(&self.http, &token).await
        {
            tracing::warn!("revoca del token Google non riuscita: {e}");
        }
        secrets::delete_api_key(TOKEN_KEY)?;
        app.state::<SettingsStore>()
            .update(serde_json::json!({ "googleCalendarEmail": "" }))?;
        *self.runtime() = Runtime::default();
        let path = app.state::<Db>().path.clone();
        tauri::async_runtime::spawn_blocking(move || {
            Store::open(path)?.forget_external_meetings(SOURCE)
        })
        .await
        .map_err(|e| anyhow!(e))??;
        let _ = app.emit(CHANGED_EVENT, ());
        Ok(())
    }

    /// Scarica gli eventi e aggiorna l'Agenda. Senza collegamento non fa nulla.
    pub async fn sync(&self, app: &AppHandle) -> Result<Option<SyncReport>> {
        let _guard = self.sync_lock.lock().await;
        if !secrets::has_api_key(TOKEN_KEY).unwrap_or(false) {
            return Ok(None);
        }
        self.runtime().syncing = true;
        let result = self.sync_inner(app).await;
        let mut rt = self.runtime();
        rt.syncing = false;
        match &result {
            Ok(report) => {
                rt.last_sync_ms = Some(now_ms());
                rt.last_report = Some(*report);
                rt.last_error = None;
            }
            Err(e) => {
                if matches!(
                    e.downcast_ref::<CalendarError>(),
                    Some(CalendarError::Revoked)
                ) {
                    rt.needs_reconnect = true;
                    rt.access = None;
                }
                rt.last_error = Some(format!("{e:#}"));
            }
        }
        drop(rt);
        let _ = app.emit(CHANGED_EVENT, ());
        result.map(Some)
    }

    async fn sync_inner(&self, app: &AppHandle) -> Result<SyncReport> {
        let settings = app.state::<SettingsStore>();
        let access = self.access_token(&settings).await?;
        let now = now_ms();
        let (from, to) = (now - PAST_MS, now + FUTURE_MS);
        let events = sv_calendar::list_events(&self.http, &access, from, to).await?;
        let language = settings.get().their_language.code().to_owned();
        let path = app.state::<Db>().path.clone();
        let report = tauri::async_runtime::spawn_blocking(move || -> Result<SyncReport> {
            let store = Store::open(path)?;
            let contacts = store.contacts()?;
            let meetings: Vec<_> = events
                .into_iter()
                .filter(|e| !e.declined)
                .filter_map(|e| to_external(e, &contacts))
                .collect();
            Ok(store.sync_external_meetings(SOURCE, from, to, &meetings, &language)?)
        })
        .await
        .map_err(|e| anyhow!(e))??;
        tracing::info!(
            added = report.added,
            updated = report.updated,
            removed = report.removed,
            "Google Calendar sincronizzato"
        );
        Ok(report)
    }

    async fn access_token(&self, settings: &SettingsStore) -> Result<String> {
        if let Some(access) = self.runtime().access.as_ref().filter(|a| a.is_fresh()) {
            return Ok(access.token.clone());
        }
        let client = Self::client(settings)?;
        let refresh_token =
            secrets::get_api_key(TOKEN_KEY)?.ok_or_else(|| anyhow!("calendario non collegato"))?;
        let access = sv_calendar::oauth::refresh(&self.http, &client, &refresh_token).await?;
        let token = access.token.clone();
        self.runtime().access = Some(access);
        Ok(token)
    }
}

/// Sincronizzazione periodica in background (solo se collegato).
pub fn spawn_periodic_sync(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_SYNC_AFTER).await;
        loop {
            let calendar = app.state::<Calendar>();
            if let Err(e) = calendar.sync(&app).await {
                tracing::warn!("sincronizzazione di Google Calendar non riuscita: {e:#}");
            }
            tokio::time::sleep(SYNC_EVERY).await;
        }
    });
}

/// Solo eventi con un link di call. Partecipanti collegati alla rubrica per
/// email (nome, ruolo e descrizione della scheda).
fn to_external(
    event: sv_calendar::CalendarEvent,
    contacts: &[sv_store::Contact],
) -> Option<ExternalMeeting> {
    let join_url = event.join_url?;
    let by_email: HashMap<String, &sv_store::Contact> = contacts
        .iter()
        .filter(|c| !c.email.is_empty())
        .map(|c| (c.email.to_lowercase(), c))
        .collect();
    let participants = event
        .attendees
        .iter()
        .map(|a| match by_email.get(&a.email.to_lowercase()) {
            Some(c) => ScheduledParticipant {
                name: c.name.clone(),
                role: c.role.clone(),
                description: c.description.clone(),
            },
            None => ScheduledParticipant {
                name: if a.name.is_empty() {
                    a.email.clone()
                } else {
                    a.name.clone()
                },
                ..Default::default()
            },
        })
        .collect();
    let name = if event.title.is_empty() {
        call_label(&join_url).to_owned()
    } else {
        event.title
    };
    Some(ExternalMeeting {
        external_id: event.id,
        name,
        notes: event.description,
        participants,
        starts_at_ms: event.starts_at_ms,
        ends_at_ms: event.ends_at_ms,
        join_url,
    })
}

/// Titolo di ripiego per un evento senza titolo.
fn call_label(url: &str) -> &'static str {
    if url.contains("meet.google.com") {
        "Google Meet"
    } else if url.contains("teams.") {
        "Microsoft Teams"
    } else {
        "Zoom"
    }
}

/// Apre un URL `https://` nel browser di sistema.
pub fn open_in_browser(url: &str) -> std::io::Result<()> {
    if !url.starts_with("https://") {
        return Err(std::io::Error::other(format!("url non consentito: {url}")));
    }
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        // `start` di cmd spezzerebbe l'URL sulle `&`.
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url).spawn().map(|_| ())
}

/// Testi della pagina mostrata nel browser dopo il consenso.
fn callback_page(language: &str) -> CallbackPage {
    let (ok_title, ok_body, err_title, err_body) = match language {
        "en" => (
            "Google Calendar connected",
            "You can close this page and go back to Sottovoce.",
            "Connection not completed",
            "Access was not granted. Go back to Sottovoce to try again.",
        ),
        "es" => (
            "Google Calendar conectado",
            "Puedes cerrar esta página y volver a Sottovoce.",
            "Conexión no completada",
            "No se concedió el acceso. Vuelve a Sottovoce para intentarlo de nuevo.",
        ),
        _ => (
            "Google Calendar collegato",
            "Puoi chiudere questa pagina e tornare a Sottovoce.",
            "Collegamento non completato",
            "L'accesso non è stato concesso. Torna a Sottovoce per riprovare.",
        ),
    };
    CallbackPage {
        success_title: ok_title.into(),
        success_body: ok_body.into(),
        error_title: err_title.into(),
        error_body: err_body.into(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(name: &str, email: &str) -> sv_store::Contact {
        sv_store::Contact {
            id: 1,
            name: name.into(),
            role: "CTO".into(),
            description: String::new(),
            email: email.into(),
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn event(join_url: Option<&str>) -> sv_calendar::CalendarEvent {
        sv_calendar::CalendarEvent {
            id: "e1".into(),
            title: String::new(),
            starts_at_ms: 0,
            ends_at_ms: 30 * 60_000,
            description: "agenda".into(),
            join_url: join_url.map(str::to_owned),
            attendees: vec![
                sv_calendar::Attendee {
                    name: "anna b.".into(),
                    email: "Anna@Example.com".into(),
                },
                sv_calendar::Attendee {
                    name: String::new(),
                    email: "marco@example.com".into(),
                },
            ],
            declined: false,
        }
    }

    #[test]
    fn imports_only_calls_and_links_contacts_by_email() {
        assert!(to_external(event(None), &[]).is_none());
        let contacts = [contact("Anna Bianchi", "anna@example.com")];
        let m = to_external(
            event(Some("https://meet.google.com/abc-defg-hij")),
            &contacts,
        )
        .unwrap();
        assert_eq!(m.name, "Google Meet");
        let names: Vec<_> = m
            .participants
            .iter()
            .map(|p| (p.name.as_str(), p.role.as_str()))
            .collect();
        assert_eq!(names, [("Anna Bianchi", "CTO"), ("marco@example.com", "")]);
    }

    #[test]
    fn refuses_non_https_urls() {
        assert!(open_in_browser("file:///etc/passwd").is_err());
    }
}
