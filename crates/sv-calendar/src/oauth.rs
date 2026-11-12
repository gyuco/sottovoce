//! OAuth 2.0 per app desktop (RFC 8252): browser di sistema, redirect su
//! loopback (`http://127.0.0.1:<porta>` o `http://localhost:<porta>`) e PKCE
//! (S256).
//!
//! L'app apre un listener locale su una porta libera, manda l'utente alla
//! pagina di consenso del provider (Google, Microsoft) e aspetta il redirect
//! con il codice, che poi scambia con access token e refresh token. Nessun
//! server intermedio.

use std::future::Future;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::error::CalendarError;

/// Endpoint e permessi di un provider OAuth.
#[derive(Debug, Clone)]
pub struct Provider {
    pub auth_url: String,
    pub token_url: String,
    /// Permessi richiesti, separati da spazi.
    pub scopes: &'static str,
    /// Permesso sul calendario che deve risultare concesso (confronto sul
    /// suffisso: Microsoft lo restituisce con o senza il prefisso Graph).
    pub calendar_scope: &'static str,
    /// Host del redirect: Google vuole l'IP di loopback, Entra ID accetta
    /// `http://localhost` con qualsiasi porta.
    pub redirect_host: &'static str,
    /// Parametri aggiuntivi della richiesta di consenso.
    pub extra_params: &'static [(&'static str, &'static str)],
    /// Il refresh ripete i permessi (Microsoft; Google li deduce dal token).
    pub refresh_with_scope: bool,
    /// Nome del provider nei messaggi.
    pub name: &'static str,
}

/// Google Calendar: sola lettura degli eventi, più l'email dell'account.
pub fn google() -> Provider {
    Provider {
        auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
        token_url: "https://oauth2.googleapis.com/token".into(),
        scopes: "openid email https://www.googleapis.com/auth/calendar.events.readonly",
        calendar_scope: "https://www.googleapis.com/auth/calendar.events.readonly",
        redirect_host: "127.0.0.1",
        // Refresh token anche se l'utente aveva già dato il consenso.
        extra_params: &[("access_type", "offline"), ("prompt", "consent")],
        refresh_with_scope: false,
        name: "Google",
    }
}

/// Microsoft 365 / Outlook (Microsoft Graph). `tenant`: `common` (account
/// aziendali e personali), `organizations`, `consumers`, ID o dominio della
/// directory per un'app registrata su un solo tenant.
pub fn microsoft(tenant: &str) -> Result<Provider, CalendarError> {
    let tenant = tenant.trim();
    let tenant = if tenant.is_empty() { "common" } else { tenant };
    if tenant.len() > 100
        || !tenant
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return Err(CalendarError::Invalid(format!(
            "tenant non valido: {tenant}"
        )));
    }
    let base = format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0");
    Ok(Provider {
        auth_url: format!("{base}/authorize"),
        token_url: format!("{base}/token"),
        scopes: "openid email profile offline_access https://graph.microsoft.com/Calendars.Read",
        calendar_scope: "Calendars.Read",
        redirect_host: "localhost",
        extra_params: &[("prompt", "select_account")],
        refresh_with_scope: true,
        name: "Microsoft",
    })
}

/// Client OAuth creato dall'utente presso il provider. Google (client "App
/// desktop") vuole anche il secret, che per un'app desktop non è
/// confidenziale ma resta comunque nel Keychain; Microsoft (client pubblico)
/// no.
#[derive(Debug, Clone)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: Option<String>,
}

impl OAuthClient {
    fn params<'a>(&'a self, mut params: Vec<(&'a str, &'a str)>) -> Vec<(&'a str, &'a str)> {
        params.push(("client_id", self.client_id.as_str()));
        if let Some(secret) = &self.client_secret {
            params.push(("client_secret", secret.as_str()));
        }
        params
    }
}

/// Testi della pagina che il browser mostra dopo il consenso.
#[derive(Debug, Clone)]
pub struct CallbackPage {
    pub success_title: String,
    pub success_body: String,
    pub error_title: String,
    pub error_body: String,
}

#[derive(Debug, Clone)]
pub struct AccessToken {
    pub token: String,
    pub expires_at: Instant,
}

impl AccessToken {
    /// Valido ancora per almeno un minuto.
    pub fn is_fresh(&self) -> bool {
        self.expires_at > Instant::now() + Duration::from_secs(60)
    }
}

#[derive(Debug, Clone)]
pub struct Authorized {
    pub refresh_token: String,
    pub access: AccessToken,
    /// Email dell'account collegato (dall'`id_token`).
    pub email: Option<String>,
}

/// Esito di un refresh. Microsoft ruota il refresh token a ogni uso: quello
/// nuovo, se c'è, va salvato al posto del vecchio.
#[derive(Debug, Clone)]
pub struct Refreshed {
    pub access: AccessToken,
    pub refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    refresh_token: Option<String>,
    id_token: Option<String>,
    #[serde(default)]
    scope: String,
}

#[derive(Deserialize)]
struct TokenError {
    error: String,
    #[serde(default)]
    error_description: String,
}

fn random_b64(bytes: usize) -> Result<String, CalendarError> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| CalendarError::Invalid(format!("random: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

/// `code_challenge` S256 del verifier.
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Listener del redirect: IPv4 sempre, IPv6 se disponibile sulla stessa
/// porta (un browser può risolvere `localhost` in `::1`).
struct Listeners {
    v4: TcpListener,
    v6: Option<TcpListener>,
}

impl Listeners {
    async fn bind() -> std::io::Result<(Self, u16)> {
        let v4 = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = v4.local_addr()?.port();
        let v6 = TcpListener::bind(("::1", port)).await.ok();
        Ok((Self { v4, v6 }, port))
    }

    async fn accept(&self) -> std::io::Result<TcpStream> {
        match &self.v6 {
            Some(v6) => tokio::select! {
                r = self.v4.accept() => r.map(|(s, _)| s),
                r = v6.accept() => r.map(|(s, _)| s),
            },
            None => self.v4.accept().await.map(|(s, _)| s),
        }
    }
}

/// Flusso completo: apre il browser, aspetta il redirect (fino a `timeout` o
/// a `cancel`) e scambia il codice. `open_browser` riceve l'URL di consenso.
pub async fn authorize(
    http: &reqwest::Client,
    provider: &Provider,
    client: &OAuthClient,
    page: &CallbackPage,
    open_browser: impl FnOnce(&str) -> std::io::Result<()>,
    cancel: impl Future<Output = ()>,
    timeout: Duration,
) -> Result<Authorized, CalendarError> {
    let (listeners, port) = Listeners::bind().await?;
    let redirect_uri = format!("http://{}:{port}", provider.redirect_host);
    let verifier = random_b64(48)?;
    let state = random_b64(16)?;
    let code_challenge = challenge(&verifier);
    let mut params = vec![
        ("client_id", client.client_id.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("response_type", "code"),
        ("scope", provider.scopes),
        ("code_challenge", code_challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("state", state.as_str()),
    ];
    params.extend_from_slice(provider.extra_params);
    let url = url::Url::parse_with_params(&provider.auth_url, &params)
        .map_err(|e| CalendarError::Invalid(e.to_string()))?;
    open_browser(url.as_str())?;

    let code = tokio::time::timeout(timeout, async {
        tokio::select! {
            () = cancel => Err(CalendarError::Cancelled),
            code = wait_for_code(&listeners, &state, page) => code,
        }
    })
    .await
    .map_err(|_| CalendarError::Timeout)??;

    let response = http
        .post(&provider.token_url)
        .form(&client.params(vec![
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
        ]))
        .send()
        .await?;
    let tokens: TokenResponse = token_response(response).await?;
    // Con il consenso granulare l'utente può togliere la spunta al calendario.
    if !has_scope(&tokens.scope, provider.calendar_scope) {
        return Err(CalendarError::MissingScope);
    }
    let refresh_token = tokens.refresh_token.ok_or_else(|| {
        CalendarError::Invalid(format!(
            "{} non ha restituito il refresh token",
            provider.name
        ))
    })?;
    Ok(Authorized {
        refresh_token,
        email: tokens.id_token.as_deref().and_then(id_token_email),
        access: AccessToken {
            token: tokens.access_token,
            expires_at: Instant::now() + Duration::from_secs(tokens.expires_in),
        },
    })
}

/// Il permesso sul calendario è tra quelli concessi (anche con prefisso,
/// es. `https://graph.microsoft.com/Calendars.Read`).
fn has_scope(granted: &str, wanted: &str) -> bool {
    granted.split_whitespace().any(|s| {
        s == wanted
            || s.strip_suffix(wanted)
                .is_some_and(|prefix| prefix.ends_with('/'))
    })
}

/// Nuovo access token dal refresh token. [`CalendarError::Revoked`] se
/// l'utente ha tolto l'accesso (o il token è scaduto): va ricollegato.
pub async fn refresh(
    http: &reqwest::Client,
    provider: &Provider,
    client: &OAuthClient,
    refresh_token: &str,
) -> Result<Refreshed, CalendarError> {
    let mut params = vec![
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];
    if provider.refresh_with_scope {
        params.push(("scope", provider.scopes));
    }
    let response = http
        .post(&provider.token_url)
        .form(&client.params(params))
        .send()
        .await?;
    let tokens: TokenResponse = token_response(response).await?;
    Ok(Refreshed {
        access: AccessToken {
            token: tokens.access_token,
            expires_at: Instant::now() + Duration::from_secs(tokens.expires_in),
        },
        refresh_token: tokens.refresh_token,
    })
}

/// Revoca il token presso Google (scollegamento). Errori ignorabili.
/// Microsoft non ha un endpoint di revoca per i client pubblici: l'accesso si
/// toglie da account.microsoft.com / myapps.microsoft.com.
pub async fn revoke_google(http: &reqwest::Client, token: &str) -> Result<(), CalendarError> {
    http.post("https://oauth2.googleapis.com/revoke")
        .form(&[("token", token)])
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn token_response(response: reqwest::Response) -> Result<TokenResponse, CalendarError> {
    let status = response.status();
    let body = response.text().await?;
    if status.is_success() {
        return serde_json::from_str(&body).map_err(|e| CalendarError::Invalid(e.to_string()));
    }
    let err: Option<TokenError> = serde_json::from_str(&body).ok();
    Err(match err {
        // Microsoft: `invalid_grant` anche per consenso mancante/da rinnovare.
        Some(e) if e.error == "invalid_grant" || e.error == "interaction_required" => {
            CalendarError::Revoked
        }
        Some(e) if e.error == "invalid_client" || e.error == "unauthorized_client" => {
            CalendarError::InvalidClient(first_line(&e.error_description))
        }
        Some(e) => CalendarError::Api {
            status: status.as_u16(),
            message: format!("{}: {}", e.error, first_line(&e.error_description)),
        },
        None => CalendarError::Api {
            status: status.as_u16(),
            message: body.chars().take(300).collect(),
        },
    })
}

/// Prima riga (Entra ID aggiunge trace id e timestamp nelle successive).
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_owned()
}

/// Email dal payload dell'`id_token` (solo per mostrarla: nessuna verifica
/// della firma, il token arriva direttamente dal provider su TLS). Microsoft
/// può non avere `email`: allora `preferred_username`.
fn id_token_email(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    ["email", "preferred_username"]
        .iter()
        .find_map(|k| value.get(k)?.as_str().filter(|s| !s.is_empty()))
        .map(str::to_owned)
}

/// Accetta connessioni finché arriva il redirect con il nostro `state`.
/// Richieste estranee (favicon, `state` diverso) ricevono 404 e si ignorano.
async fn wait_for_code(
    listeners: &Listeners,
    state: &str,
    page: &CallbackPage,
) -> Result<String, CalendarError> {
    loop {
        let mut stream = listeners.accept().await?;
        let Ok(Ok(target)) =
            tokio::time::timeout(Duration::from_secs(5), read_target(&mut stream)).await
        else {
            continue;
        };
        let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
            respond(&mut stream, "404 Not Found", "").await;
            continue;
        };
        let param = |key: &str| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        if param("state").as_deref() != Some(state) {
            respond(&mut stream, "404 Not Found", "").await;
            continue;
        }
        if let Some(error) = param("error") {
            respond(
                &mut stream,
                "200 OK",
                &html(&page.error_title, &page.error_body),
            )
            .await;
            return Err(CalendarError::Denied(error));
        }
        if let Some(code) = param("code") {
            respond(
                &mut stream,
                "200 OK",
                &html(&page.success_title, &page.success_body),
            )
            .await;
            return Ok(code);
        }
        respond(&mut stream, "400 Bad Request", "").await;
    }
}

/// Percorso della richiesta (`/?code=…`) dalla prima riga HTTP.
async fn read_target(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&buf);
    let line = head.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) if target.starts_with('/') => Ok(target.to_owned()),
        _ => Err(std::io::Error::other("richiesta non valida")),
    }
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn html(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Sottovoce</title>\
         <style>body{{font-family:-apple-system,system-ui,sans-serif;display:grid;place-items:center;\
         min-height:90vh;margin:0;background:#f6f6f7;color:#18181b}}main{{max-width:28rem;padding:2rem;\
         text-align:center}}h1{{font-size:1.4rem}}p{{color:#52525b}}\
         @media(prefers-color-scheme:dark){{body{{background:#18181b;color:#fafafa}}p{{color:#a1a1aa}}}}</style>\
         </head><body><main><h1>{}</h1><p>{}</p></main></body></html>",
        escape(title),
        escape(body)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_base64url_sha256() {
        // `printf 'verifier-di-prova' | openssl dgst -sha256 -binary | base64`
        // senza padding e con l'alfabeto URL-safe.
        assert_eq!(
            challenge("verifier-di-prova"),
            "bPPq6pE71yiqfdOYRu6SmWuGX1qYwaoLaM7ycB0jISw"
        );
    }

    #[test]
    fn checks_granted_scope() {
        let ms = "openid email https://graph.microsoft.com/Calendars.Read profile";
        assert!(has_scope(ms, "Calendars.Read"));
        assert!(has_scope("Calendars.Read", "Calendars.Read"));
        assert!(!has_scope(
            "https://graph.microsoft.com/Calendars.ReadBasic",
            "Calendars.Read"
        ));
        assert!(!has_scope(
            "https://graph.microsoft.com/xCalendars.Read",
            "Calendars.Read"
        ));
        let google = google();
        assert!(has_scope(
            &format!("openid {}", google.calendar_scope),
            google.calendar_scope
        ));
        assert!(!has_scope("openid email", google.calendar_scope));
    }

    #[test]
    fn microsoft_tenant_is_validated() {
        assert!(microsoft("").unwrap().auth_url.contains("/common/"));
        assert!(microsoft("contoso.onmicrosoft.com").is_ok());
        assert!(microsoft("1b2c3d4e-0000-1111-2222-333344445555").is_ok());
        assert!(microsoft("evil.com/x?").is_err());
    }

    #[test]
    fn reads_email_from_id_token() {
        let ms = URL_SAFE_NO_PAD.encode(br#"{"preferred_username":"marco@contoso.com"}"#);
        assert_eq!(
            id_token_email(&format!("h.{ms}.s")).as_deref(),
            Some("marco@contoso.com")
        );
        let payload = URL_SAFE_NO_PAD.encode(br#"{"email":"anna@example.com","sub":"1"}"#);
        assert_eq!(
            id_token_email(&format!("h.{payload}.s")).as_deref(),
            Some("anna@example.com")
        );
        assert_eq!(id_token_email("garbage"), None);
    }

    fn page() -> CallbackPage {
        CallbackPage {
            success_title: "ok".into(),
            success_body: "chiudi".into(),
            error_title: "no".into(),
            error_body: "<rifiutato>".into(),
        }
    }

    async fn get(port: u16, target: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(format!("GET {target} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    #[tokio::test]
    async fn loopback_ignores_strangers_and_returns_code() {
        let (listeners, port) = Listeners::bind().await.unwrap();
        let page = page();
        let client = async {
            assert!(get(port, "/favicon.ico").await.starts_with("HTTP/1.1 404"));
            assert!(
                get(port, "/?code=x&state=other")
                    .await
                    .starts_with("HTTP/1.1 404")
            );
            let ok = get(port, "/?code=4%2Fabc&state=S").await;
            assert!(ok.starts_with("HTTP/1.1 200") && ok.contains("chiudi"));
        };
        let (code, ()) = tokio::join!(wait_for_code(&listeners, "S", &page), client);
        assert_eq!(code.unwrap(), "4/abc");
    }

    #[tokio::test]
    async fn loopback_reports_denied_consent() {
        let (listeners, port) = Listeners::bind().await.unwrap();
        let page = page();
        let client = async {
            let body = get(port, "/?error=access_denied&state=S").await;
            assert!(body.contains("&lt;rifiutato&gt;"));
        };
        let (result, ()) = tokio::join!(wait_for_code(&listeners, "S", &page), client);
        assert!(matches!(result, Err(CalendarError::Denied(e)) if e == "access_denied"));
    }
}
