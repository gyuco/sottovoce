//! Controllo aggiornamenti: confronta la build installata con l'ultima release
//! pubblicata su GitHub.
//!
//! Le release in bozza non sono visibili all'API anonima, quindi finché non si
//! pubblica una release l'app non segnala nulla. Nelle build locali lo sha del
//! commit non è impostato (`SOTTOVOCE_BUILD_SHA` lo mette la CI) e il controllo
//! viene saltato.

use std::time::Duration;

use serde::Serialize;

/// Release pubblicate del repo dell'app.
const RELEASES_URL: &str = "https://api.github.com/repos/gyuco/sottovoce/releases?per_page=10";

/// Prefisso consentito per l'apertura di una pagina nel browser.
const RELEASE_PREFIX: &str = "https://github.com/gyuco/sottovoce/releases/";

/// Sha del commit con cui è stata compilata la build corrente.
const BUILD_SHA: Option<&str> = option_env!("SOTTOVOCE_BUILD_SHA");

/// Istante di compilazione (RFC 3339 UTC), confrontabile in modo lessicografico
/// con `published_at` dell'API perché hanno lo stesso formato.
const BUILD_TIME: Option<&str> = option_env!("SOTTOVOCE_BUILD_TIME");

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// Versione dichiarata nella release (`v0.1.0-abc1234` → `0.1.0`).
    pub version: String,
    /// Sha corto della build disponibile, confrontabile con [BUILD_SHA].
    pub build: String,
    pub notes: String,
    pub url: String,
    /// Data di pubblicazione in formato RFC 3339.
    pub published_at: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
}

/// Sha corto di una build: primi 7 caratteri.
fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

/// Tag `v0.1.0-abc1234` → versione (`0.1.0`) e sha corto (`abc1234`).
fn split_tag(tag: &str) -> Option<(&str, &str)> {
    let (version, build) = tag.rsplit_once('-')?;
    let version = version.strip_prefix('v').unwrap_or(version);
    (!version.is_empty() && !build.is_empty()).then_some((version, build))
}

/// Ultima release pubblicata più recente della build installata.
pub async fn check() -> Option<UpdateInfo> {
    let current = short(BUILD_SHA?);

    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .user_agent(concat!("sottovoce/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;

    let releases: Vec<Release> = client
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;

    let latest = releases
        .into_iter()
        .find(|release| !release.draft && release.published_at.is_some())?;
    let (version, build) = split_tag(&latest.tag_name)?;

    // Stessa build già installata: niente da segnalare.
    if build == current {
        return None;
    }

    // Release pubblicata prima che questa build esistesse: è più vecchia.
    if let (Some(built), Some(published)) = (BUILD_TIME, latest.published_at.as_deref())
        && published <= built
    {
        return None;
    }

    tracing::info!(
        version,
        build,
        "aggiornamento disponibile: {}",
        latest.html_url
    );

    Some(UpdateInfo {
        version: version.to_string(),
        build: build.to_string(),
        notes: latest.body.unwrap_or_default(),
        url: latest.html_url,
        published_at: latest.published_at,
    })
}

/// Apre la pagina di una release nel browser predefinito.
pub fn open_release(url: &str) -> Result<(), String> {
    if !url.starts_with(RELEASE_PREFIX) {
        return Err(format!("url non consentito: {url}"));
    }
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("apertura del browser non riuscita: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_di_release() {
        assert_eq!(split_tag("v0.1.0-abc1234"), Some(("0.1.0", "abc1234")));
        assert_eq!(split_tag("0.1.0-abc1234"), Some(("0.1.0", "abc1234")));
        assert_eq!(
            split_tag("v1.2.3-beta.1-abc1234"),
            Some(("1.2.3-beta.1", "abc1234"))
        );
        assert_eq!(split_tag("senza-sha"), Some(("senza", "sha")));
        assert_eq!(split_tag("niente"), None);
        assert_eq!(split_tag("v0.1.0-"), None);
        assert_eq!(split_tag("-abc1234"), None);
    }

    #[test]
    fn sha_corto() {
        assert_eq!(short("0123456789abcdef"), "0123456");
        assert_eq!(short("abc"), "abc");
        assert_eq!(short(""), "");
    }

    #[test]
    fn apri_solo_release_del_repo() {
        assert!(open_release("https://example.com/releases/v1").is_err());
        assert!(open_release("https://github.com/altro/repo/releases/v1").is_err());
    }
}
