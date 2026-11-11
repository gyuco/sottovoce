//! Lingua di una frase già trascritta (italiano, inglese, spagnolo,
//! francese, tedesco) dalle parole funzionali. Parakeet non dichiara la
//! lingua: serve a saltare la traduzione delle frasi già nella lingua
//! dell'utente e a stimare la lingua del meeting.
//!
//! Le parole comuni a più lingue romanze ("la", "un", "che", "del") stanno
//! solo nella lista italiana: in caso di dubbio vince l'italiano.

const IT: &[&str] = &[
    "il", "lo", "la", "gli", "le", "di", "del", "della", "dei", "delle", "che", "è", "non", "per",
    "un", "una", "uno", "sono", "con", "come", "anche", "questo", "questa", "ma", "nel", "nella",
    "al", "alla", "ci", "si", "mi", "ti", "ho", "hai", "abbiamo", "siamo", "perché", "quando",
    "dove", "cosa", "qual", "quale", "molto", "più", "già", "ancora", "allora", "quindi", "poi",
    "bene", "grazie", "sì", "va", "fare", "stato",
];

const EN: &[&str] = &[
    "the", "and", "is", "are", "to", "of", "you", "that", "it", "we", "this", "what", "have",
    "for", "with", "be", "was", "were", "they", "will", "would", "can", "could", "should", "about",
    "there", "their", "our", "your", "not", "do", "does", "did", "just", "so", "if", "how", "why",
    "when", "where", "which", "who", "yes", "thanks", "okay", "think", "know", "going", "an", "my",
];

const ES: &[&str] = &[
    "el", "los", "las", "que", "es", "está", "están", "pero", "muy", "también", "esto", "esta",
    "este", "eso", "yo", "usted", "nosotros", "hay", "hemos", "tenemos", "puede", "porque",
    "cuando", "donde", "qué", "cómo", "gracias", "bueno", "entonces", "ya", "y", "hacer", "vamos",
    "creo", "ahora", "todo", "todos", "algo", "nada", "hola", "para", "por", "estamos", "sí",
];

const FR: &[&str] = &[
    "les", "des", "est", "et", "je", "vous", "nous", "ils", "elle", "pas", "avec", "pour", "dans",
    "sur", "mais", "où", "très", "aussi", "bien", "oui", "merci", "alors", "donc", "être", "avoir",
    "fait", "cette", "ce", "ces", "au", "aux", "du", "leur", "sont", "avons", "peut", "comme",
    "quand", "pourquoi", "quoi", "à", "déjà", "après", "même", "été", "tout", "rien", "bonjour",
    "on", "que",
];

const DE: &[&str] = &[
    "der", "die", "das", "und", "ist", "nicht", "ich", "sie", "wir", "ein", "eine", "einen", "den",
    "dem", "zu", "mit", "auf", "für", "von", "im", "auch", "aber", "wie", "wo", "wann", "warum",
    "ja", "nein", "danke", "haben", "habe", "sind", "wird", "werden", "kann", "können", "noch",
    "schon", "dann", "sehr", "gut", "bitte", "hallo", "dass", "oder", "wenn", "jetzt",
];

const LANGS: [(&str, &[&str]); 5] = [("it", IT), ("en", EN), ("es", ES), ("fr", FR), ("de", DE)];

/// Lettere che compaiono quasi solo in una lingua.
fn letter_language(c: char) -> Option<usize> {
    match c {
        'ì' | 'ò' | 'à' | 'è' | 'ù' => Some(0),
        'ñ' | '¿' | '¡' | 'á' | 'í' | 'ó' | 'ú' => Some(2),
        'ç' | 'â' | 'ê' | 'î' | 'ô' | 'û' | 'ë' | 'ï' | 'œ' => Some(3),
        'ä' | 'ö' | 'ü' | 'ß' => Some(4),
        _ => None,
    }
}

/// Codice della lingua indicata chiaramente dalle parole funzionali,
/// `None` se il testo non basta o è ambiguo.
pub fn guess(text: &str) -> Option<&'static str> {
    let mut score = [0usize; 5];
    for word in text
        .split(|c: char| !c.is_alphabetic() && c != '\'')
        .filter(|w| !w.is_empty())
    {
        let w = word.to_lowercase();
        // Elisioni: "l'altro", "dell'anno" (italiano), "qu'il", "j'ai" (francese).
        let (head, tail) = w.split_once('\'').unwrap_or((&w, ""));
        if !tail.is_empty() {
            match head {
                "l" | "dell" | "all" | "nell" | "un" | "c" | "d" | "sull" if tail != "est" => {
                    score[0] += 1;
                    continue;
                }
                "c" | "qu" | "j" | "n" | "s" | "m" | "jusqu" | "lorsqu" => {
                    score[3] += 1;
                    continue;
                }
                _ => {}
            }
        }
        let mut known = false;
        for (i, (_, words)) in LANGS.iter().enumerate() {
            if words.contains(&w.as_str()) {
                score[i] += 1;
                known = true;
            }
        }
        if !known && let Some(i) = w.chars().find_map(letter_language) {
            score[i] += 1;
        }
    }
    let best = *score.iter().max()?;
    if best == 0 || score.iter().filter(|&&s| s == best).count() > 1 {
        return None;
    }
    score.iter().position(|&s| s == best).map(|i| LANGS[i].0)
}

#[cfg(test)]
mod tests {
    use super::guess;

    #[test]
    fn italian_and_english_sentences() {
        assert_eq!(guess("Qual è lo stato del progetto?"), Some("it"));
        assert_eq!(
            guess("Buongiorno a tutti, iniziamo la riunione."),
            Some("it")
        );
        assert_eq!(guess("C'è ancora l'ultimo punto da chiudere"), Some("it"));
        assert_eq!(
            guess("We finished the API migration last week."),
            Some("en")
        );
        assert_eq!(
            guess("What do you think about the data contract?"),
            Some("en")
        );
    }

    #[test]
    fn unclear_text_is_none() {
        assert_eq!(guess(""), None);
        assert_eq!(guess("Okay."), Some("en"));
        assert_eq!(guess("Kubernetes, Terraform."), None);
    }

    #[test]
    fn spanish_french_german_sentences() {
        assert_eq!(
            guess("¿Qué opinas del contrato? Creo que está bien."),
            Some("es")
        );
        assert_eq!(guess("Hola a todos, ya tenemos los datos."), Some("es"));
        assert_eq!(
            guess("C'est très bien, nous avons fini la migration."),
            Some("fr")
        );
        assert_eq!(
            guess("Je pense qu'il faut vérifier les données."),
            Some("fr")
        );
        assert_eq!(guess("Ich glaube, das ist noch nicht fertig."), Some("de"));
        assert_eq!(guess("Wir haben die Daten schon geprüft."), Some("de"));
    }
}
