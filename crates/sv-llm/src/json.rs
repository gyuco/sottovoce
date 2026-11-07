use serde::de::DeserializeOwned;

use crate::LlmError;

/// Legge JSON da una risposta LLM, tollerando fence markdown (```json … ```)
/// e testo prima o dopo l'oggetto.
pub fn parse_json<T: DeserializeOwned>(text: &str) -> Result<T, LlmError> {
    let text = text.trim();
    let first_err = match serde_json::from_str(text) {
        Ok(v) => return Ok(v),
        Err(e) => e,
    };

    if let Some(inner) = fenced(text)
        && let Ok(v) = serde_json::from_str(inner)
    {
        return Ok(v);
    }

    if let Some(inner) = outer_json(text)
        && let Ok(v) = serde_json::from_str(inner)
    {
        return Ok(v);
    }

    Err(LlmError::InvalidJson(first_err.to_string()))
}

/// Contenuto del primo blocco ``` … ```, senza l'eventuale tag di lingua.
fn fenced(text: &str) -> Option<&str> {
    let start = text.find("```")? + 3;
    let rest = &text[start..];
    let body_start = rest.find('\n').map_or(0, |i| i + 1);
    let rest = &rest[body_start..];
    let end = rest.find("```")?;
    Some(rest[..end].trim())
}

/// Dalla prima `{`/`[` all'ultima chiusura corrispondente.
fn outer_json(text: &str) -> Option<&str> {
    let start = text.find(['{', '['])?;
    let close = if text[start..].starts_with('{') {
        '}'
    } else {
        ']'
    };
    let end = text.rfind(close)?;
    (end > start).then(|| &text[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Assist {
        say_this: Vec<String>,
    }

    fn ok(text: &str) -> Assist {
        parse_json(text).expect(text)
    }

    #[test]
    fn plain() {
        assert_eq!(ok(r#"{"say_this":["a"]}"#).say_this, ["a"]);
    }

    #[test]
    fn fenced_with_language_tag() {
        let text = "Ecco:\n```json\n{\"say_this\": [\"a\", \"b\"]}\n```\nFine.";
        assert_eq!(ok(text).say_this, ["a", "b"]);
    }

    #[test]
    fn fenced_without_language_tag() {
        assert_eq!(
            ok("```\n{\"say_this\": []}\n```").say_this,
            Vec::<String>::new()
        );
    }

    #[test]
    fn surrounding_text() {
        let text = "Certo! {\"say_this\": [\"x\"]} Spero aiuti.";
        assert_eq!(ok(text).say_this, ["x"]);
    }

    #[test]
    fn array_root() {
        let v: Vec<u32> = parse_json("risultato: [1, 2, 3].").unwrap();
        assert_eq!(v, [1, 2, 3]);
    }

    #[test]
    fn invalid() {
        assert!(matches!(
            parse_json::<Assist>("nessun json qui"),
            Err(LlmError::InvalidJson(_))
        ));
        assert!(matches!(
            parse_json::<Assist>("{\"say_this\": [\"a\""),
            Err(LlmError::InvalidJson(_))
        ));
    }
}
