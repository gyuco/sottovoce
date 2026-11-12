//! Invio del riepilogo di un meeting per email.
//!
//! Il frontend compone oggetto, HTML e testo semplice (etichette tradotte, vedi
//! `src/lib/email.ts`); qui si validano i destinatari e si spedisce il
//! messaggio `multipart/alternative` in due modi:
//! - **SMTP** con il server configurato nelle impostazioni; la password sta
//!   nel Keychain (account `smtp`) e non torna mai al frontend;
//! - **bozza** nel client di posta predefinito: un file `.eml` con
//!   `X-Unsent: 1` (Mail e Outlook lo aprono come messaggio da inviare),
//!   scritto nella cache dell'app e rimosso alla bozza successiva.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use lettre::message::header::{self, HeaderName, HeaderValue, Headers};
use lettre::message::{Mailbox, Mailboxes, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::Deserialize;

use crate::export::sanitize_file_name;
use crate::secrets;
use crate::settings::{SMTP_KEY, Settings, SmtpSecurity};

const MAX_RECIPIENTS: usize = 50;
const MAX_SUBJECT_CHARS: usize = 300;
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const SMTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Messaggio composto dal frontend.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingEmail {
    pub to: Vec<String>,
    pub subject: String,
    pub html: String,
    pub text: String,
}

/// Destinatari validati e senza doppioni, oggetto su una riga.
struct Checked {
    to: Mailboxes,
    subject: String,
    html: String,
    text: String,
}

fn check(email: OutgoingEmail) -> Result<Checked> {
    let mut seen = Vec::new();
    let mut to = Mailboxes::new();
    for raw in &email.to {
        let address = raw.trim();
        if address.is_empty() {
            continue;
        }
        if !sv_store::is_valid_email(address) {
            bail!("indirizzo email non valido: {address}");
        }
        let key = address.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        to.push(Mailbox::new(
            None,
            address.parse().context("indirizzo email non valido")?,
        ));
    }
    if seen.is_empty() {
        bail!("indica almeno un destinatario");
    }
    if seen.len() > MAX_RECIPIENTS {
        bail!("troppi destinatari (massimo {MAX_RECIPIENTS})");
    }
    let subject: String = email
        .subject
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_SUBJECT_CHARS)
        .collect();
    if subject.is_empty() {
        bail!("l'oggetto è vuoto");
    }
    if email.html.len() + email.text.len() > MAX_BODY_BYTES {
        bail!("il messaggio è troppo grande");
    }
    Ok(Checked {
        to,
        subject,
        html: email.html,
        text: email.text,
    })
}

/// Mittente dalle impostazioni; `None` se l'indirizzo non è impostato.
fn sender(settings: &Settings) -> Result<Option<Mailbox>> {
    let address = settings.email_from.trim();
    if address.is_empty() {
        return Ok(None);
    }
    if !sv_store::is_valid_email(address) {
        bail!("indirizzo del mittente non valido: {address}");
    }
    let name = settings.email_from_name.trim();
    Ok(Some(Mailbox::new(
        (!name.is_empty()).then(|| name.to_owned()),
        address
            .parse()
            .context("indirizzo del mittente non valido")?,
    )))
}

fn body(checked: &Checked) -> MultiPart {
    MultiPart::alternative_plain_html(checked.text.clone(), checked.html.clone())
}

/// Server SMTP pronto all'uso. Legge la password dal Keychain: chiamarla
/// fuori dal thread principale.
fn transport(settings: &Settings) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let host = settings.smtp_host.trim();
    if host.is_empty() {
        bail!("configura il server SMTP in Impostazioni → Email");
    }
    let user = match settings.smtp_user.trim() {
        "" => settings.email_from.trim(),
        user => user,
    };
    if user.is_empty() {
        bail!("indica l'indirizzo del mittente o l'utente SMTP");
    }
    let password = secrets::get_api_key(SMTP_KEY)?
        .ok_or_else(|| anyhow!("salva la password SMTP in Impostazioni → Email"))?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let builder = match settings.smtp_security {
        SmtpSecurity::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host),
        SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(host),
    }
    .with_context(|| format!("server SMTP non valido: {host}"))?;
    Ok(builder
        .port(settings.smtp_port)
        .credentials(Credentials::new(user.to_owned(), password))
        .timeout(Some(SMTP_TIMEOUT))
        .build())
}

/// Spedisce il messaggio con il server SMTP configurato.
pub async fn send(settings: &Settings, email: OutgoingEmail) -> Result<usize> {
    let checked = check(email)?;
    let from = sender(settings)?
        .ok_or_else(|| anyhow!("indica l'indirizzo del mittente in Impostazioni → Email"))?;
    let mut message = Message::builder()
        .from(from)
        .subject(checked.subject.clone());
    for mailbox in checked.to.iter() {
        message = message.to(mailbox.clone());
    }
    let message = message
        .multipart(body(&checked))
        .context("messaggio non valido")?;
    let settings = settings.clone();
    let transport = tauri::async_runtime::spawn_blocking(move || transport(&settings))
        .await
        .map_err(|e| anyhow!("{e}"))??;
    transport
        .send(message)
        .await
        .map_err(smtp_error)
        .context("invio non riuscito")?;
    let count = checked.to.iter().count();
    tracing::info!(recipients = count, "riepilogo inviato via email");
    Ok(count)
}

/// Prova connessione e autenticazione senza inviare nulla.
pub async fn test(settings: &Settings) -> Result<()> {
    sender(settings)?
        .ok_or_else(|| anyhow!("indica l'indirizzo del mittente in Impostazioni → Email"))?;
    let settings = settings.clone();
    let transport = tauri::async_runtime::spawn_blocking(move || transport(&settings))
        .await
        .map_err(|e| anyhow!("{e}"))??;
    match transport.test_connection().await {
        Ok(true) => Ok(()),
        Ok(false) => bail!("il server SMTP non risponde"),
        Err(e) => Err(smtp_error(e)).context("connessione al server SMTP non riuscita"),
    }
}

/// Errore SMTP leggibile: le risposte del server (es. "535 auth failed")
/// sono più utili del solo tipo di errore.
fn smtp_error(e: lettre::transport::smtp::Error) -> anyhow::Error {
    let mut message = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(inner) = source {
        message.push_str(": ");
        message.push_str(&inner.to_string());
        source = inner.source();
    }
    anyhow!(message)
}

/// Messaggio `.eml` da aprire come bozza: `X-Unsent: 1`, mittente solo se
/// impostato (altrimenti lo sceglie il client di posta).
fn draft_bytes(settings: &Settings, email: OutgoingEmail) -> Result<(String, Vec<u8>)> {
    let checked = check(email)?;
    let mut headers = Headers::new();
    if let Some(from) = sender(settings)? {
        headers.set(header::From::from(Mailboxes::from(from)));
    }
    headers.set(header::To::from(checked.to.clone()));
    headers.set(header::Subject::from(checked.subject.clone()));
    headers.set(header::Date::now());
    headers.set(header::MIME_VERSION_1_0);
    headers.insert_raw(HeaderValue::new(
        HeaderName::new_from_ascii_str("X-Unsent"),
        "1".to_owned(),
    ));
    let mut bytes = headers.to_string().into_bytes();
    bytes.extend_from_slice(&body(&checked).formatted());
    Ok((checked.subject, bytes))
}

/// Scrive la bozza in `dir` (svuotata prima: una sola bozza alla volta) e la
/// apre con il client di posta predefinito.
pub fn open_draft(dir: &Path, settings: &Settings, email: OutgoingEmail) -> Result<PathBuf> {
    let (subject, bytes) = draft_bytes(settings, email)?;
    if dir.exists() {
        for entry in std::fs::read_dir(dir)?.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    std::fs::create_dir_all(dir).context("cartella delle bozze non creata")?;
    let path = dir.join(format!("{}.eml", sanitize_file_name(&subject)));
    std::fs::write(&path, bytes).context("bozza non scritta")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    std::process::Command::new("open")
        .arg(&path)
        .spawn()
        .context("client di posta non aperto")?;
    tracing::info!("bozza del riepilogo aperta nel client di posta");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn email(to: &[&str]) -> OutgoingEmail {
        OutgoingEmail {
            to: to.iter().map(|s| (*s).to_owned()).collect(),
            subject: "Riepilogo:\r\n  Sprint review".into(),
            html: "<p>Ciao <b>team</b></p>".into(),
            text: "Ciao team".into(),
        }
    }

    #[test]
    fn checks_recipients_and_subject() {
        let ok = check(email(&["a@b.it", " A@B.it ", "", "c@d.com"])).unwrap();
        assert_eq!(ok.to.iter().count(), 2);
        assert_eq!(ok.subject, "Riepilogo: Sprint review");
        assert!(check(email(&[])).is_err());
        assert!(check(email(&["  "])).is_err());
        assert!(check(email(&["a@b.it\r\nBcc: x@y.z"])).is_err());
        let many: Vec<String> = (0..51).map(|i| format!("p{i}@x.it")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(check(email(&many)).is_err());
    }

    #[test]
    fn draft_is_an_unsent_multipart_message() {
        let settings = Settings {
            email_from: "io@example.com".into(),
            email_from_name: "Giù Rossi".into(),
            ..Settings::default()
        };
        let (subject, bytes) = draft_bytes(&settings, email(&["anna@example.com"])).unwrap();
        assert_eq!(subject, "Riepilogo: Sprint review");
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("X-Unsent: 1\r\n"));
        assert!(text.contains("To: anna@example.com\r\n"));
        assert!(text.contains("From: "));
        assert!(text.contains("multipart/alternative"));
        assert!(text.contains("text/html"));
        // Nessun a capo dell'oggetto finisce tra le intestazioni.
        assert!(!text.contains("Riepilogo:\r\n"));

        // Senza mittente la bozza si crea comunque (lo sceglie il client).
        let (_, bytes) = draft_bytes(&Settings::default(), email(&["a@b.it"])).unwrap();
        assert!(!String::from_utf8(bytes).unwrap().contains("From: "));
    }

    #[test]
    fn rejects_bad_sender() {
        let settings = Settings {
            email_from: "non-valido".into(),
            ..Settings::default()
        };
        assert!(sender(&settings).is_err());
        assert!(sender(&Settings::default()).unwrap().is_none());
    }
}
