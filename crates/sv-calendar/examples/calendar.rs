//! Prova del collegamento a un calendario da terminale: consenso nel browser,
//! refresh del token, poi eventi dei prossimi giorni con il link della call.
//!
//! GOOGLE_CLIENT_ID=... GOOGLE_CLIENT_SECRET=... \
//!     cargo run -p sv-calendar --example calendar -- google [giorni]
//! MICROSOFT_CLIENT_ID=... [MICROSOFT_TENANT=common] \
//!     cargo run -p sv-calendar --example calendar -- microsoft [giorni]

use std::time::Duration;

use sv_calendar::{CallbackPage, OAuthClient};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let which = args.next().unwrap_or_else(|| "google".into());
    let days: i64 = args.next().and_then(|d| d.parse().ok()).unwrap_or(7);
    let (provider, client) = match which.as_str() {
        "google" => (
            sv_calendar::oauth::google(),
            OAuthClient {
                client_id: std::env::var("GOOGLE_CLIENT_ID")?,
                client_secret: Some(std::env::var("GOOGLE_CLIENT_SECRET")?),
            },
        ),
        "microsoft" => (
            sv_calendar::oauth::microsoft(&std::env::var("MICROSOFT_TENANT").unwrap_or_default())?,
            OAuthClient {
                client_id: std::env::var("MICROSOFT_CLIENT_ID")?,
                client_secret: None,
            },
        ),
        other => return Err(format!("provider sconosciuto: {other} (google | microsoft)").into()),
    };
    let page = CallbackPage {
        success_title: "Collegato".into(),
        success_body: "Puoi chiudere questa pagina.".into(),
        error_title: "Non collegato".into(),
        error_body: "Accesso non concesso.".into(),
    };
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = reqwest::Client::new();
    let authorized = sv_calendar::oauth::authorize(
        &http,
        &provider,
        &client,
        &page,
        |url| {
            println!("Apri nel browser se non si apre da solo:\n{url}\n");
            std::process::Command::new("open")
                .arg(url)
                .spawn()
                .map(|_| ())
        },
        std::future::pending(),
        Duration::from_secs(300),
    )
    .await?;
    println!("collegato come {:?}", authorized.email);

    // Il refresh token funziona (come all'avvio successivo dell'app).
    let refreshed =
        sv_calendar::oauth::refresh(&http, &provider, &client, &authorized.refresh_token).await?;
    println!(
        "refresh ok (token ruotato: {})",
        refreshed.refresh_token.is_some()
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    let (from, to) = (now - 86_400_000, now + days * 86_400_000);
    let token = &refreshed.access.token;
    let events = if which == "google" {
        sv_calendar::list_events(&http, token, from, to).await?
    } else {
        sv_calendar::microsoft::list_events(&http, token, from, to, authorized.email.as_deref())
            .await?
    };
    println!(
        "{} eventi con orario nei prossimi {days} giorni\n",
        events.len()
    );
    for e in events {
        println!(
            "{} · {} min · {}{}",
            sv_calendar::time::format_utc(e.starts_at_ms),
            (e.ends_at_ms - e.starts_at_ms) / 60_000,
            e.title,
            if e.declined { " (rifiutato)" } else { "" }
        );
        if let Some(url) = &e.join_url {
            println!("  link: {url}");
        }
        for a in &e.attendees {
            println!("  - {} <{}>", a.name, a.email);
        }
    }
    if which == "google" {
        let _ = sv_calendar::oauth::revoke_google(&http, &authorized.refresh_token).await;
    }
    Ok(())
}
