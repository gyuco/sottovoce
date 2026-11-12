//! Sincronizzazione con i calendari (Google Calendar, Outlook / Microsoft
//! 365): le riunioni con un link di call (Meet, Teams, Zoom) diventano
//! riunioni programmate nell'Agenda.
//!
//! - Collegamento: OAuth desktop (browser di sistema + redirect su loopback +
//!   PKCE, vedi `sv_calendar::oauth`). Il client OAuth lo crea l'utente presso
//!   il provider (Google Cloud, Microsoft Entra): ID client nei settings,
//!   client secret (solo Google) e refresh token nel Keychain. Nessun server
//!   intermedio.
//! - Sincronizzazione: all'avvio, ogni 10 minuti, dopo il collegamento e a
//!   richiesta. Finestra da 1 giorno fa a 30 giorni avanti; esclusi eventi
//!   senza link di call e inviti rifiutati. Evento `scheduled:changed` alla UI.
//! - Scollegamento: token cancellato (e revocato presso Google; Microsoft non
//!   ha revoca per i client pubblici), riunioni importate rimosse tranne
//!   quelle modificate dall'utente.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sv_calendar::{AccessToken, CalendarError, CallbackPage, OAuthClient};
use sv_store::{ExternalMeeting, ScheduledParticipant, Store, SyncReport};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::secrets;
use crate::settings::{Settings, SettingsStore};
use crate::state::Db;

/// Evento alla UI: l'Agenda va ricaricata.
pub const CHANGED_EVENT: &str = "scheduled:changed";

const PAST_MS: i64 = 24 * 60 * 60 * 1000;
const FUTURE_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const SYNC_EVERY: Duration = Duration::from_secs(10 * 60);
const FIRST_SYNC_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CalendarProvider {
    Google,
    Microsoft,
}

impl CalendarProvider {
    pub const ALL: [Self; 2] = [Self::Google, Self::Microsoft];

    /// Valore di `scheduled_meetings.source`.
    pub fn source(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Microsoft => "microsoft",
        }
    }

    /// Account Keychain del refresh token (vedi `secrets::KNOWN_PROVIDERS`).
    pub fn token_key(self) -> &'static str {
        match self {
            Self::Google => "google-calendar",
            Self::Microsoft => "microsoft-calendar",
        }
    }

    /// Account Keychain del client secret; Microsoft usa un client pubblico.
    fn secret_key(self) -> Option<&'static str> {
        match self {
            Self::Google => Some("google-client-secret"),
            Self::Microsoft => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Google => "Google Calendar",
            Self::Microsoft => "Outlook",
        }
    }

    fn client_id(self, settings: &Settings) -> String {
        match self {
            Self::Google => settings.google_client_id.trim().to_owned(),
            Self::Microsoft => settings.microsoft_client_id.trim().to_owned(),
        }
    }

    fn email(self, settings: &Settings) -> String {
        match self {
            Self::Google => settings.google_calendar_email.clone(),
            Self::Microsoft => settings.microsoft_calendar_email.clone(),
        }
    }

    /// Chiave dei settings con l'email dell'account collegato.
    fn email_setting(self) -> &'static str {
        match self {
            Self::Google => "googleCalendarEmail",
            Self::Microsoft => "microsoftCalendarEmail",
        }
    }

    fn oauth(self, settings: &Settings) -> Result<sv_calendar::Provider> {
        Ok(match self {
            Self::Google => sv_calendar::oauth::google(),
            Self::Microsoft => sv_calendar::oauth::microsoft(&settings.microsoft_tenant_id)?,
        })
    }
}

/// Il refresh token di un calendario: lo scrive solo il collegamento OAuth.
pub fn is_token_key(key: &str) -> bool {
    CalendarProvider::ALL.iter().any(|p| p.token_key() == key)
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarStatus {
    /// ID client (e, per Google, client secret) presenti.
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
    runtime: Mutex<HashMap<CalendarProvider, Runtime>>,
    /// Una sincronizzazione alla volta per provider.
    sync_locks: HashMap<CalendarProvider, tokio::sync::Mutex<()>>,
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
            sync_locks: CalendarProvider::ALL
                .iter()
                .map(|p| (*p, tokio::sync::Mutex::new(())))
                .collect(),
        })
    }

    fn with_runtime<T>(&self, provider: CalendarProvider, f: impl FnOnce(&mut Runtime) -> T) -> T {
        let mut map = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        f(map.entry(provider).or_default())
    }

    fn sync_lock(&self, provider: CalendarProvider) -> &tokio::sync::Mutex<()> {
        &self.sync_locks[&provider]
    }

    pub fn status(&self, provider: CalendarProvider, settings: &SettingsStore) -> CalendarStatus {
        let settings = settings.get();
        let configured = !provider.client_id(&settings).is_empty()
            && provider
                .secret_key()
                .is_none_or(|key| secrets::has_api_key(key).unwrap_or(false));
        let connected = secrets::has_api_key(provider.token_key()).unwrap_or(false);
        self.with_runtime(provider, |rt| CalendarStatus {
            configured,
            connected,
            email: provider.email(&settings),
            connecting: rt.connecting.is_some(),
            syncing: rt.syncing,
            last_sync_ms: rt.last_sync_ms,
            last_report: rt.last_report,
            last_error: rt.last_error.clone(),
            needs_reconnect: rt.needs_reconnect,
        })
    }

    fn client(provider: CalendarProvider, settings: &Settings) -> Result<OAuthClient> {
        let client_id = provider.client_id(settings);
        if client_id.is_empty() {
            bail!("ID client di {} mancante", provider.label());
        }
        let client_secret = match provider.secret_key() {
            Some(key) => Some(
                secrets::get_api_key(key)?
                    .ok_or_else(|| anyhow!("client secret di {} mancante", provider.label()))?,
            ),
            None => None,
        };
        Ok(OAuthClient {
            client_id,
            client_secret,
        })
    }

    /// Apre il consenso nel browser e aspetta il ritorno (max 5 minuti o
    /// [`Calendar::cancel_connect`]); poi sincronizza.
    pub async fn connect(
        &self,
        provider: CalendarProvider,
        app: &AppHandle,
        ui_language: &str,
    ) -> Result<()> {
        let settings_store = app.state::<SettingsStore>();
        let settings = settings_store.get();
        let client = Self::client(provider, &settings)?;
        let oauth = provider.oauth(&settings)?;
        let cancel = Arc::new(Notify::new());
        self.with_runtime(provider, |rt| {
            if rt.connecting.is_some() {
                bail!("collegamento già in corso");
            }
            rt.connecting = Some(cancel.clone());
            Ok(())
        })?;
        let result = sv_calendar::oauth::authorize(
            &self.http,
            &oauth,
            &client,
            &callback_page(provider.label(), ui_language),
            open_in_browser,
            cancel.notified(),
            CONNECT_TIMEOUT,
        )
        .await;
        self.with_runtime(provider, |rt| rt.connecting = None);
        let authorized = match result {
            Ok(a) => a,
            Err(CalendarError::Cancelled) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        secrets::set_api_key(provider.token_key(), &authorized.refresh_token)?;
        settings_store.update(serde_json::json!({
            provider.email_setting(): authorized.email.unwrap_or_default()
        }))?;
        self.with_runtime(provider, |rt| {
            rt.access = Some(authorized.access);
            rt.needs_reconnect = false;
            rt.last_error = None;
        });
        self.sync(provider, app).await?;
        Ok(())
    }

    pub fn cancel_connect(&self, provider: CalendarProvider) {
        self.with_runtime(provider, |rt| {
            if let Some(cancel) = &rt.connecting {
                cancel.notify_one();
            }
        });
    }

    /// Cancella il token (revocandolo presso Google) e le riunioni importate
    /// non modificate.
    pub async fn disconnect(&self, provider: CalendarProvider, app: &AppHandle) -> Result<()> {
        let _guard = self.sync_lock(provider).lock().await;
        if provider == CalendarProvider::Google
            && let Ok(Some(token)) = secrets::get_api_key(provider.token_key())
            && let Err(e) = sv_calendar::oauth::revoke_google(&self.http, &token).await
        {
            tracing::warn!("revoca del token Google non riuscita: {e}");
        }
        secrets::delete_api_key(provider.token_key())?;
        app.state::<SettingsStore>()
            .update(serde_json::json!({ provider.email_setting(): "" }))?;
        self.with_runtime(provider, |rt| *rt = Runtime::default());
        let path = app.state::<Db>().path.clone();
        tauri::async_runtime::spawn_blocking(move || {
            Store::open(path)?.forget_external_meetings(provider.source())
        })
        .await
        .map_err(|e| anyhow!(e))??;
        let _ = app.emit(CHANGED_EVENT, ());
        Ok(())
    }

    /// Scarica gli eventi e aggiorna l'Agenda. Senza collegamento non fa nulla.
    pub async fn sync(
        &self,
        provider: CalendarProvider,
        app: &AppHandle,
    ) -> Result<Option<SyncReport>> {
        let _guard = self.sync_lock(provider).lock().await;
        if !secrets::has_api_key(provider.token_key()).unwrap_or(false) {
            return Ok(None);
        }
        self.with_runtime(provider, |rt| rt.syncing = true);
        let result = self.sync_inner(provider, app).await;
        self.with_runtime(provider, |rt| {
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
        });
        let _ = app.emit(CHANGED_EVENT, ());
        result.map(Some)
    }

    async fn sync_inner(&self, provider: CalendarProvider, app: &AppHandle) -> Result<SyncReport> {
        let settings = app.state::<SettingsStore>().get();
        let access = self.access_token(provider, &settings).await?;
        let now = now_ms();
        let (from, to) = (now - PAST_MS, now + FUTURE_MS);
        let events = match provider {
            CalendarProvider::Google => {
                sv_calendar::list_events(&self.http, &access, from, to).await?
            }
            CalendarProvider::Microsoft => {
                let email = provider.email(&settings);
                let me = (!email.is_empty()).then_some(email.as_str());
                sv_calendar::microsoft::list_events(&self.http, &access, from, to, me).await?
            }
        };
        let language = settings.their_language.code().to_owned();
        let path = app.state::<Db>().path.clone();
        let report = tauri::async_runtime::spawn_blocking(move || -> Result<SyncReport> {
            let store = Store::open(path)?;
            let contacts = store.contacts()?;
            let meetings: Vec<_> = events
                .into_iter()
                .filter(|e| !e.declined)
                .filter_map(|e| to_external(e, &contacts))
                .collect();
            Ok(store.sync_external_meetings(provider.source(), from, to, &meetings, &language)?)
        })
        .await
        .map_err(|e| anyhow!(e))??;
        tracing::info!(
            provider = provider.source(),
            added = report.added,
            updated = report.updated,
            removed = report.removed,
            "calendario sincronizzato"
        );
        Ok(report)
    }

    async fn access_token(
        &self,
        provider: CalendarProvider,
        settings: &Settings,
    ) -> Result<String> {
        let cached = self.with_runtime(provider, |rt| {
            rt.access
                .as_ref()
                .filter(|a| a.is_fresh())
                .map(|a| a.token.clone())
        });
        if let Some(token) = cached {
            return Ok(token);
        }
        let client = Self::client(provider, settings)?;
        let oauth = provider.oauth(settings)?;
        let refresh_token = secrets::get_api_key(provider.token_key())?
            .ok_or_else(|| anyhow!("calendario non collegato"))?;
        let refreshed =
            sv_calendar::oauth::refresh(&self.http, &oauth, &client, &refresh_token).await?;
        // Microsoft ruota il refresh token: il vecchio smette di valere.
        if let Some(next) = refreshed.refresh_token.filter(|t| *t != refresh_token) {
            secrets::set_api_key(provider.token_key(), &next)?;
        }
        let token = refreshed.access.token.clone();
        self.with_runtime(provider, |rt| rt.access = Some(refreshed.access));
        Ok(token)
    }
}

/// Sincronizzazione periodica in background (solo dei calendari collegati).
pub fn spawn_periodic_sync(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_SYNC_AFTER).await;
        loop {
            let calendar = app.state::<Calendar>();
            for provider in CalendarProvider::ALL {
                if let Err(e) = calendar.sync(provider, &app).await {
                    tracing::warn!(
                        provider = provider.source(),
                        "sincronizzazione del calendario non riuscita: {e:#}"
                    );
                }
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
fn callback_page(label: &str, language: &str) -> CallbackPage {
    let (ok_title, ok_body, err_title, err_body) = match language {
        "en" => (
            format!("{label} connected"),
            "You can close this page and go back to Sottovoce.",
            "Connection not completed",
            "Access was not granted. Go back to Sottovoce to try again.",
        ),
        "es" => (
            format!("{label} conectado"),
            "Puedes cerrar esta página y volver a Sottovoce.",
            "Conexión no completada",
            "No se concedió el acceso. Vuelve a Sottovoce para intentarlo de nuevo.",
        ),
        _ => (
            format!("{label} collegato"),
            "Puoi chiudere questa pagina e tornare a Sottovoce.",
            "Collegamento non completato",
            "L'accesso non è stato concesso. Torna a Sottovoce per riprovare.",
        ),
    };
    CallbackPage {
        success_title: ok_title,
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
    fn token_keys_are_known_and_protected() {
        for provider in CalendarProvider::ALL {
            assert!(is_token_key(provider.token_key()));
            assert!(secrets::KNOWN_PROVIDERS.contains(&provider.token_key()));
            if let Some(key) = provider.secret_key() {
                assert!(secrets::KNOWN_PROVIDERS.contains(&key));
                assert!(!is_token_key(key));
            }
        }
        assert!(!is_token_key("smtp"));
    }

    #[test]
    fn refuses_non_https_urls() {
        assert!(open_in_browser("file:///etc/passwd").is_err());
    }
}
