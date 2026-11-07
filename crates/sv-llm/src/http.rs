//! Parti HTTP comuni ai client: retry, mappatura degli errori.

use std::time::Duration;

use reqwest::StatusCode;

use crate::LlmError;

/// Timeout di default per richiesta (retry inclusi).
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Attesa massima fra due tentativi, anche se `Retry-After` chiede di più.
const MAX_RETRY_DELAY: Duration = Duration::from_secs(5);
/// Caratteri del corpo di errore conservati in [`LlmError::Http`].
const ERROR_BODY_MAX: usize = 500;

/// Retry su 429 e 5xx con backoff esponenziale. Mai su altri 4xx.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base_delay: Duration::from_millis(500),
        }
    }
}

/// Client reqwest con lo user agent dell'app (rustls + ring).
pub(crate) fn http_client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(concat!("sottovoce/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// Invia la richiesta ritentando su 429/5xx. Ritorna l'ultimo stato
/// ricevuto (non necessariamente di successo), il corpo e i tentativi.
pub(crate) async fn send_with_retry(
    retry: RetryPolicy,
    build: impl Fn() -> reqwest::RequestBuilder,
) -> Result<(StatusCode, String, u32), LlmError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let resp = build().send().await.map_err(LlmError::from_reqwest)?;
        let status = resp.status();
        let retry_after = retry_after(&resp);
        let text = resp.text().await.map_err(LlmError::from_reqwest)?;

        let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
        if !retryable || attempt > retry.max_retries {
            return Ok((status, text, attempt));
        }
        let backoff = retry.base_delay * 2u32.pow(attempt - 1);
        let delay = retry_after
            .unwrap_or_default()
            .max(backoff)
            .min(MAX_RETRY_DELAY);
        tracing::debug!(%status, attempt, ?delay, "richiesta LLM ritentata");
        tokio::time::sleep(delay).await;
    }
}

pub(crate) fn status_error(status: StatusCode, body: String) -> LlmError {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => LlmError::Unauthorized,
        StatusCode::TOO_MANY_REQUESTS => LlmError::RateLimited,
        _ => LlmError::Http {
            status: status.as_u16(),
            body: truncate(body),
        },
    }
}

fn truncate(mut s: String) -> String {
    if let Some((i, _)) = s.char_indices().nth(ERROR_BODY_MAX) {
        s.truncate(i);
        s.push('…');
    }
    s
}

fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let secs: u64 = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs))
}
