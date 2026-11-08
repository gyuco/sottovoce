//! Lingua di una frase già trascritta, italiano o inglese, dalle parole
//! funzionali. Parakeet non dichiara la lingua: serve a saltare la
//! traduzione delle frasi già nella lingua dell'utente.

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

/// `Some("it")` / `Some("en")` se le parole funzionali indicano chiaramente
/// una lingua, altrimenti `None`.
pub fn guess(text: &str) -> Option<&'static str> {
    let mut it = 0;
    let mut en = 0;
    for word in text
        .split(|c: char| !c.is_alphabetic() && c != '\'')
        .filter(|w| !w.is_empty())
    {
        let w = word.to_lowercase();
        // Elisioni italiane: "l'altro", "dell'anno", "c'è".
        let (head, tail) = w.split_once('\'').unwrap_or((&w, ""));
        if !tail.is_empty()
            && matches!(
                head,
                "l" | "dell" | "all" | "nell" | "un" | "c" | "d" | "sull"
            )
        {
            it += 1;
            continue;
        }
        if IT.contains(&w.as_str()) {
            it += 1;
        }
        if EN.contains(&w.as_str()) {
            en += 1;
        }
        if w.chars()
            .any(|c| matches!(c, 'à' | 'è' | 'é' | 'ì' | 'ò' | 'ù'))
            && !IT.contains(&w.as_str())
        {
            it += 1;
        }
    }
    match it.cmp(&en) {
        std::cmp::Ordering::Greater => Some("it"),
        std::cmp::Ordering::Less => Some("en"),
        std::cmp::Ordering::Equal => None,
    }
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
}
