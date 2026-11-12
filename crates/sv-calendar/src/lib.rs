//! Riunioni dal calendario dell'utente.
//!
//! - [`oauth`]: OAuth 2.0 per app desktop (browser di sistema, redirect su
//!   loopback, PKCE). Nessun server intermedio: i token restano sulla macchina.
//! - [`google`]: eventi di Google Calendar (sola lettura) in [`CalendarEvent`].
//! - [`microsoft`]: eventi di Outlook / Microsoft 365 via Microsoft Graph.
//! - [`link`]: link delle call (Meet, Teams, Zoom) e pulizia delle descrizioni.
//!
//! Il crate non dipende da Tauri né salva nulla: Keychain e database li
//! gestisce l'app.

mod error;
pub mod google;
pub mod link;
pub mod microsoft;
pub mod oauth;
pub mod time;

pub use error::CalendarError;
pub use google::{Attendee, CalendarEvent, list_events};
pub use link::{clean_description, find_call_link};
pub use oauth::{AccessToken, Authorized, CallbackPage, OAuthClient, Provider, Refreshed};
