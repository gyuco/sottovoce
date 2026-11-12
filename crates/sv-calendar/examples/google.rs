//! Prova del collegamento a Google Calendar da terminale: consenso nel
//! browser, poi eventi dei prossimi giorni con il link della call.
//!
//! GOOGLE_CLIENT_ID=... GOOGLE_CLIENT_SECRET=... \
//!     cargo run -p sv-calendar --example google -- [giorni]

use std::time::Duration;

use sv_calendar::{CallbackPage, OAuthClient};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let days: i64 = std::env::args()
        .nth(1)
        .and_then(|d| d.parse().ok())
        .unwrap_or(7);
    let client = OAuthClient {
        client_id: std::env::var("GOOGLE_CLIENT_ID")?,
        client_secret: std::env::var("GOOGLE_CLIENT_SECRET")?,
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
    let access = sv_calendar::oauth::refresh(&http, &client, &authorized.refresh_token).await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    let events = sv_calendar::list_events(
        &http,
        &access.token,
        now - 86_400_000,
        now + days * 86_400_000,
    )
    .await?;
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
    let _ = sv_calendar::oauth::revoke(&http, &authorized.refresh_token).await;
    Ok(())
}
