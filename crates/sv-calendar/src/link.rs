//! Link delle call (Meet, Teams, Zoom) e pulizia della descrizione degli
//! inviti (HTML e testo standard di Google/Teams).

/// Primo link di una call nel testo: Google Meet, Microsoft Teams o Zoom.
pub fn find_call_link(text: &str) -> Option<String> {
    let text = decode_entities(text);
    let mut rest = text.as_str();
    while let Some(at) = rest.find("https://") {
        let candidate = &rest[at..];
        let end = candidate
            .find(|c: char| c.is_whitespace() || "<>\"'()[]{}|\\^`".contains(c))
            .unwrap_or(candidate.len());
        let link = candidate[..end].trim_end_matches(['.', ',', ';', ':']);
        if is_call_link(link) {
            return Some(link.to_owned());
        }
        rest = &candidate[end.max(8)..];
    }
    None
}

fn is_call_link(link: &str) -> bool {
    let Ok(url) = url::Url::parse(link) else {
        return false;
    };
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let path = url.path();
    match host.as_str() {
        "meet.google.com" => path.len() > 1,
        "teams.microsoft.com" => path.starts_with("/l/meetup-join/") || path.starts_with("/meet/"),
        "teams.live.com" => path.starts_with("/meet/"),
        _ => {
            (host == "zoom.us" || host.ends_with(".zoom.us"))
                && (path.starts_with("/j/") || path.starts_with("/my/"))
        }
    }
}

/// Entità HTML più comuni negli inviti.
fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Descrizione leggibile: niente tag HTML, niente blocco standard con i
/// dettagli di accesso (Google `-::~:~::~`, Teams riga di trattini bassi).
pub fn clean_description(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '<' {
            text.push(c);
            continue;
        }
        let mut tag = String::new();
        for c in chars.by_ref() {
            if c == '>' {
                break;
            }
            tag.push(c);
        }
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "br" | "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3"
        ) {
            text.push('\n');
        }
    }
    let text = decode_entities(&text);

    let mut lines = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.contains("-::~:~::~") || (line.len() >= 20 && line.chars().all(|c| c == '_')) {
            break;
        }
        // Al massimo una riga vuota di seguito.
        if line.is_empty() && lines.last().is_none_or(|l: &&str| l.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_call_links() {
        assert_eq!(
            find_call_link("Partecipa: https://meet.google.com/abc-defg-hij.").as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        let teams = "<a href=\"https://teams.microsoft.com/l/meetup-join/19%3ameeting_X%40thread.v2/0?context=%7b%7d&amp;x=1\">Join</a>";
        assert_eq!(
            find_call_link(teams).as_deref(),
            Some(
                "https://teams.microsoft.com/l/meetup-join/19%3ameeting_X%40thread.v2/0?context=%7b%7d&x=1"
            )
        );
        assert!(find_call_link("https://us02web.zoom.us/j/123?pwd=x").is_some());
        assert!(find_call_link("https://example.com https://meet.google.com/").is_none());
        assert!(find_call_link("https://teams.microsoft.com/v2/").is_none());
        assert!(find_call_link("").is_none());
    }

    #[test]
    fn cleans_descriptions() {
        let google = "Agenda:<br>1. budget<br><br><br>2. rischi\n\n-::~:~::~:~::~:~::-\nPartecipa con Google Meet: https://meet.google.com/x";
        assert_eq!(clean_description(google), "Agenda:\n1. budget\n\n2. rischi");
        let teams = "<p>Ordine del giorno &amp; note</p>\n________________________________________________________________________________\nRiunione di Microsoft Teams";
        assert_eq!(clean_description(teams), "Ordine del giorno & note");
        assert_eq!(clean_description("  "), "");
    }
}
