//! Codice locale del progetto: verifica della cartella e contesto per il
//! live assist.
//!
//! Il percorso è solo un'informazione del contesto meeting (non finisce nei
//! settings): qui si legge **in sola lettura** un sottoinsieme dei file
//! sorgente per dare al modello un indice del progetto e righe di codice
//! reali con cui verificare ciò che viene detto in call. Niente scritture,
//! niente indici persistenti: ogni sessione riparte da zero.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

/// Caratteri massimi dell'indice dei file nel prompt.
const MAX_OUTLINE_CHARS: usize = 900;
/// Caratteri massimi del blocco `CODEBASE` nel prompt.
const MAX_BLOCK_CHARS: usize = 1600;
/// Caratteri massimi degli estratti di codice.
const MAX_EXCERPTS_CHARS: usize = 700;
/// File massimi censiti (oltre: `truncated`).
const MAX_FILES: usize = 8_000;
/// Tempo massimo per censire la cartella.
const SCAN_BUDGET: Duration = Duration::from_millis(400);
/// File letti al massimo per cercare gli estratti.
const MAX_SEARCH_FILES: usize = 500;
/// Byte letti al massimo per cercare gli estratti.
const MAX_SEARCH_BYTES: usize = 6 * 1024 * 1024;
/// File più grandi: non si leggono (di solito generati/minificati).
const MAX_FILE_BYTES: u64 = 400 * 1024;
/// Tempo massimo per la ricerca degli estratti.
const SEARCH_BUDGET: Duration = Duration::from_millis(150);
/// Righe di codice per estratto (2 sopra, 2 sotto).
const SNIPPET_RADIUS: usize = 2;
/// Caratteri massimi per riga negli estratti.
const MAX_LINE_CHARS: usize = 160;

/// Cartelle che non sono codice del progetto.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".next",
    ".nuxt",
    ".turbo",
    ".cache",
    ".venv",
    "__pycache__",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    "vendor",
    "Pods",
    "DerivedData",
];

/// Parole troppo comuni per valere come termine da cercare nel codice.
const STOPWORDS: &[&str] = &[
    // italiano
    "abbiamo",
    "adesso",
    "allora",
    "anche",
    "avere",
    "bene",
    "boh",
    "buono",
    "capito",
    "certo",
    "che",
    "chi",
    "come",
    "con",
    "cosa",
    "così",
    "della",
    "delle",
    "dei",
    "degli",
    "dentro",
    "dobbiamo",
    "dopo",
    "dove",
    "dovrebbe",
    "essere",
    "facciamo",
    "fare",
    "fatto",
    "già",
    "grande",
    "guardate",
    "invece",
    "loro",
    "male",
    "meglio",
    "meno",
    "molto",
    "nella",
    "nelle",
    "negli",
    "non",
    "nostro",
    "oggi",
    "ogni",
    "ottimo",
    "perché",
    "però",
    "più",
    "possono",
    "possiamo",
    "prima",
    "secondo",
    "quando",
    "questo",
    "questa",
    "quello",
    "quella",
    "quindi",
    "siamo",
    "solo",
    "sono",
    "sopra",
    "sotto",
    "subito",
    "sul",
    "sulla",
    "tutto",
    "tutti",
    "ultimo",
    "una",
    "uno",
    "vediamo",
    "vero",
    // inglese
    "about",
    "after",
    "again",
    "also",
    "and",
    "another",
    "anything",
    "are",
    "because",
    "been",
    "before",
    "being",
    "between",
    "both",
    "but",
    "can",
    "could",
    "does",
    "doing",
    "done",
    "each",
    "even",
    "every",
    "everything",
    "from",
    "going",
    "good",
    "great",
    "have",
    "here",
    "just",
    "know",
    "like",
    "made",
    "make",
    "maybe",
    "more",
    "most",
    "much",
    "need",
    "nothing",
    "okay",
    "only",
    "other",
    "over",
    "really",
    "right",
    "should",
    "something",
    "still",
    "such",
    "sure",
    "than",
    "that",
    "their",
    "them",
    "then",
    "there",
    "these",
    "they",
    "thing",
    "think",
    "this",
    "those",
    "through",
    "very",
    "want",
    "were",
    "what",
    "when",
    "where",
    "which",
    "while",
    "will",
    "with",
    "would",
    "yeah",
    // spagnolo
    "ahora",
    "algo",
    "bueno",
    "como",
    "cuando",
    "desde",
    "donde",
    "entre",
    "este",
    "esta",
    "esto",
    "hacer",
    "mejor",
    "mucho",
    "para",
    "pero",
    "porque",
    "puede",
    "sobre",
    "también",
    "tenemos",
    "todo",
    "todos",
];

/// Un file per estensione linguistica.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageCount {
    pub name: &'static str,
    pub files: usize,
}

/// Esito della verifica di una cartella di codice (mostrato in UI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodePathInfo {
    /// Percorso canonico (assoluto) se esiste; altrimenti quello digitato.
    pub path: String,
    pub exists: bool,
    pub is_dir: bool,
    /// File regolari censiti (esclusi `.git`, `node_modules`, …).
    pub files: usize,
    /// Vero se il censimento si è fermato prima di finire.
    pub truncated: bool,
    /// Linguaggi in ordine di diffusione.
    pub languages: Vec<LanguageCount>,
    /// Indice compatto dei file sorgente.
    pub outline: String,
}

impl CodePathInfo {
    fn missing(path: String) -> Self {
        Self {
            path,
            exists: false,
            is_dir: false,
            files: 0,
            truncated: false,
            languages: Vec::new(),
            outline: String::new(),
        }
    }
}

/// Verifica una cartella senza tenerla in memoria: usata dalla UI.
pub fn inspect(input: &str) -> CodePathInfo {
    scan(input).0
}

/// Cartella di codice pronta per costruire il contesto dei prompt.
#[derive(Debug)]
pub struct Codebase {
    root: PathBuf,
    info: CodePathInfo,
    /// File di testo in ordine di importanza (prima i sorgenti).
    sources: Vec<PathBuf>,
}

impl Codebase {
    /// Apre la cartella; errore se non esiste o non è una directory.
    pub fn open(input: &str) -> Result<Self, String> {
        let (info, mut sources) = scan(input);
        if !info.exists {
            return Err(format!("percorso non trovato: {}", info.path));
        }
        if !info.is_dir {
            return Err(format!("non è una cartella: {}", info.path));
        }
        let root = PathBuf::from(&info.path);
        sources.sort_by_cached_key(|p| priority(&relative(&root, p)));
        Ok(Self {
            root,
            info,
            sources,
        })
    }

    /// Blocco `CODEBASE` per il prompt: indice del progetto più gli estratti
    /// che contengono i termini citati in call. `None` se non c'è niente da
    /// dire (non dovrebbe capitare: l'indice c'è sempre).
    pub fn context_block(&self, terms: &[String]) -> Option<String> {
        let project = self
            .root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.info.path.clone());
        let langs: Vec<&str> = self.info.languages.iter().take(4).map(|l| l.name).collect();

        let mut out = format!("CODEBASE (local project \"{project}\"");
        if !langs.is_empty() {
            out.push_str(", ");
            out.push_str(&langs.join(", "));
        }
        out.push_str("; read from the user's machine, ground truth for technical claims)\n");
        out.push_str(&format!("Files: {}", self.info.files));
        if self.info.truncated {
            out.push('+');
        }
        out.push('\n');
        if !self.info.outline.is_empty() {
            out.push_str("Source outline:\n");
            out.push_str(&self.info.outline);
            if !self.info.outline.ends_with('\n') {
                out.push('\n');
            }
        }
        if let Some(excerpts) = self.excerpts(terms) {
            out.push_str("Relevant excerpts (file:line):\n");
            out.push_str(&excerpts);
            out.push('\n');
        }
        Some(clip(out.trim_end(), MAX_BLOCK_CHARS))
    }

    /// Righe di codice intorno ai termini, con il nome del file. Vuoto se
    /// nessun termine compare (meglio niente che rumore).
    fn excerpts(&self, terms: &[String]) -> Option<String> {
        let terms: Vec<String> = terms
            .iter()
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| t.chars().count() >= 2)
            .collect();
        if terms.is_empty() {
            return None;
        }

        // Prima i file il cui nome cita un termine (economico), poi gli altri.
        let mut named = Vec::new();
        let mut rest = Vec::new();
        for (i, path) in self.sources.iter().enumerate() {
            let rel = relative(&self.root, path).to_ascii_lowercase();
            if terms.iter().any(|t| rel.contains(t.as_str())) {
                named.push(i);
            } else {
                rest.push(i);
            }
        }

        let named_count = named.len();
        let started = Instant::now();
        let mut out = String::new();
        let (mut read_files, mut read_bytes) = (0, 0);
        for (pos, idx) in named.into_iter().chain(rest).enumerate() {
            if out.chars().count() >= MAX_EXCERPTS_CHARS
                || read_files >= MAX_SEARCH_FILES
                || started.elapsed() > SEARCH_BUDGET
            {
                break;
            }
            let path = &self.sources[idx];
            let Ok(meta) = fs::metadata(path) else {
                continue;
            };
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
            let Ok(data) = fs::read(path) else {
                continue;
            };
            read_files += 1;
            read_bytes += data.len();
            if read_bytes > MAX_SEARCH_BYTES {
                break;
            }
            let text = String::from_utf8_lossy(&data);
            let lines: Vec<&str> = text.lines().collect();
            let lower = text.to_ascii_lowercase();
            let hits: Vec<usize> = lower
                .lines()
                .enumerate()
                .filter(|(_, line)| terms.iter().any(|t| line.contains(t.as_str())))
                .map(|(i, _)| i)
                .collect();
            let rel = relative(&self.root, path);
            if hits.is_empty() {
                // File nominato in call ma senza il termine nel testo: mostra
                // l'inizio, sufficiente per orientarsi e verificare.
                if pos < named_count && !lines.is_empty() {
                    let end = lines.len().min(SNIPPET_RADIUS * 2 + 1);
                    push_snippet(&mut out, &rel, &lines, 0, end);
                }
                continue;
            }
            let mut snippets = 0;
            let mut done_until = 0;
            for hit in hits {
                if hit < done_until {
                    continue;
                }
                if snippets == 2 || out.chars().count() >= MAX_EXCERPTS_CHARS {
                    break;
                }
                let start = hit.saturating_sub(SNIPPET_RADIUS);
                let end = (hit + SNIPPET_RADIUS + 1).min(lines.len());
                push_snippet(&mut out, &rel, &lines, start, end);
                snippets += 1;
                done_until = end;
            }
        }

        let out = out.trim_end();
        (!out.is_empty()).then(|| clip(out, MAX_EXCERPTS_CHARS))
    }
}

/// Aggiunge a `out` le righe `start..end` con il riferimento `file:riga`.
fn push_snippet(out: &mut String, rel: &str, lines: &[&str], start: usize, end: usize) {
    out.push_str(&format!("{rel}:{}\n", start + 1));
    for line in &lines[start..end] {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        out.push_str("  ");
        out.push_str(&clip(line, MAX_LINE_CHARS));
        out.push('\n');
    }
    out.push('\n');
}

/// Censimento della cartella: riutilizzato da `inspect` e `Codebase::open`.
fn scan(input: &str) -> (CodePathInfo, Vec<PathBuf>) {
    let raw = expand_home(input);
    let mut info = CodePathInfo::missing(raw.display().to_string());
    let Ok(meta) = fs::metadata(&raw) else {
        return (info, Vec::new());
    };
    info.exists = true;
    info.is_dir = meta.is_dir();
    if !info.is_dir {
        return (info, Vec::new());
    }
    let root = fs::canonicalize(&raw).unwrap_or(raw);
    info.path = root.display().to_string();

    let started = Instant::now();
    let mut stack = vec![root.clone()];
    let mut sources = Vec::new();
    let mut languages: BTreeMap<&'static str, usize> = BTreeMap::new();
    while let Some(dir) = stack.pop() {
        if started.elapsed() > SCAN_BUDGET || info.files >= MAX_FILES {
            info.truncated = true;
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if started.elapsed() > SCAN_BUDGET || info.files >= MAX_FILES {
                info.truncated = true;
                break;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            // Niente symlink: evita cicli e file fuori dal progetto.
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                if !SKIP_DIRS.contains(&name.as_ref()) {
                    stack.push(path);
                }
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            info.files += 1;
            let rel = relative(&root, &path);
            if let Some(language) = language(&rel) {
                *languages.entry(language).or_default() += 1;
                sources.push(path);
            }
        }
    }

    let mut languages: Vec<LanguageCount> = languages
        .into_iter()
        .map(|(name, files)| LanguageCount { name, files })
        .collect();
    // Prima i linguaggi veri, poi configurazione e documentazione.
    let rank = |name: &str| u8::from(matches!(name, "Config" | "Markdown"));
    languages.sort_by(|a, b| {
        rank(a.name)
            .cmp(&rank(b.name))
            .then_with(|| b.files.cmp(&a.files))
            .then_with(|| a.name.cmp(b.name))
    });
    info.languages = languages;

    sources.sort_by_cached_key(|p| priority(&relative(&root, p)));
    let mut outline = String::new();
    for path in &sources {
        let rel = relative(&root, path);
        if outline.chars().count() + rel.chars().count() + 1 > MAX_OUTLINE_CHARS {
            outline.push_str("…\n");
            break;
        }
        outline.push_str(&rel);
        outline.push('\n');
    }
    info.outline = outline;

    (info, sources)
}

/// Termini da cercare nel codice, estratti dalle frasi appena dette.
///
/// Prima gli identificatori (snake_case, camelCase, percorsi, nomi di file),
/// poi i nomi propri/acronimi, infine le parole lunghe: le più "codice" non
/// devono perdere il posto per parole comuni.
pub fn extract_terms<'a>(texts: impl IntoIterator<Item = &'a str>, max: usize) -> Vec<String> {
    let mut strong = Vec::new();
    let mut names = Vec::new();
    let mut weak = Vec::new();
    let mut seen = HashSet::new();
    for text in texts {
        for raw in text.split(|c: char| {
            !(c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '/' | '-' | '@'))
        }) {
            let token = raw.trim_matches(|c: char| !c.is_alphanumeric());
            let len = token.chars().count();
            if !(3..=60).contains(&len) {
                continue;
            }
            let lower = token.to_ascii_lowercase();
            if STOPWORDS.contains(&lower.as_str()) || token.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            if !seen.insert(lower) {
                continue;
            }
            let has_digit = token.chars().any(|c| c.is_ascii_digit());
            let has_lower = token.chars().any(|c| c.is_lowercase());
            let camel = token
                .chars()
                .zip(token.chars().skip(1))
                .any(|(a, b)| a.is_lowercase() && b.is_uppercase());
            let code_shape = token.contains('_')
                || token.contains('/')
                || token.contains("::")
                || (token.contains('.') && len > 4)
                || camel
                || (has_digit && has_lower);
            let name_shape = token.chars().next().is_some_and(char::is_uppercase)
                || (len <= 6 && token.chars().all(|c| c.is_uppercase()));
            if code_shape {
                strong.push(token.to_owned());
            } else if name_shape {
                names.push(token.to_owned());
            } else if len >= 6 {
                weak.push(token.to_owned());
            }
        }
    }
    strong
        .into_iter()
        .chain(names)
        .chain(weak)
        .take(max)
        .collect()
}

/// `~/…` → home dell'utente; il resto resta com'è.
fn expand_home(input: &str) -> PathBuf {
    // Incollando da Finder o dal terminale capita di avere virgolette.
    let input = input.trim().trim_matches(|c| c == '"' || c == '\'');
    if input == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home);
    }
    if let Some(rest) = input.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(input)
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Ordine di importanza: prima i sorgenti, poi la configurazione e i doc,
/// a parità di profondità prima i file in alto.
fn priority(rel: &str) -> (u8, usize, String) {
    let language = language(rel);
    let score = match language {
        Some("Config" | "Markdown") => 2,
        Some(_) if is_code_dir(rel) => 0,
        Some(_) => 1,
        None => 3,
    };
    (score, rel.matches('/').count(), rel.to_ascii_lowercase())
}

fn is_code_dir(rel: &str) -> bool {
    [
        "src/",
        "crates/",
        "app/",
        "lib/",
        "packages/",
        "internal/",
        "cmd/",
    ]
    .iter()
    .any(|prefix| rel.starts_with(prefix))
}

/// Linguaggio (per le statistiche) di un file di testo; `None` = binario o
/// non interessante.
fn language(rel: &str) -> Option<&'static str> {
    let ext = Path::new(rel).extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "Rust",
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "py" | "pyi" => "Python",
        "go" => "Go",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "m" | "mm" => "C/C++",
        "cs" => "C#",
        "rb" => "Ruby",
        "php" => "PHP",
        "sql" => "SQL",
        "sh" | "bash" | "zsh" | "fish" => "Shell",
        "html" | "htm" => "HTML",
        "css" | "scss" | "sass" | "less" => "CSS",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "astro" => "Astro",
        "lua" => "Lua",
        "dart" => "Dart",
        "scala" => "Scala",
        "ex" | "exs" => "Elixir",
        "hs" => "Haskell",
        "r" => "R",
        "jl" => "Julia",
        "clj" | "cljs" => "Clojure",
        "tf" => "Terraform",
        "graphql" | "gql" => "GraphQL",
        "proto" => "Protobuf",
        "md" | "mdx" => "Markdown",
        "json" | "jsonc" | "toml" | "yaml" | "yml" => "Config",
        _ => return None,
    })
}

/// Primi `max` caratteri (non byte), con "…" se troncato.
fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max.saturating_sub(1)) {
        Some((i, _)) if s.chars().count() > max => format!("{}…", &s[..i]),
        _ => s.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sv-codebase-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        fs::write(
            dir.join("src/server.ts"),
            "export function createServer(port: number) {\n  return listen(port);\n}\n",
        )
        .unwrap();
        fs::write(dir.join("src/old.rs"), "fn legacy_helper() {}\n").unwrap();
        fs::write(dir.join("README.md"), "# Demo\n").unwrap();
        fs::write(dir.join("node_modules/pkg/index.js"), "ignored();\n").unwrap();
        fs::write(dir.join("logo.png"), [0x89, 0x50, 0x4e, 0x47]).unwrap();
        dir
    }

    #[test]
    fn inspect_counts_only_project_files() {
        let dir = project("inspect");
        let info = inspect(dir.to_str().unwrap());
        assert!(info.exists && info.is_dir);
        assert_eq!(info.files, 4, "node_modules e file nascosti esclusi");
        assert!(
            info.languages
                .iter()
                .any(|l| l.name == "TypeScript" && l.files == 1)
        );
        assert_eq!(info.languages.last().unwrap().name, "Markdown");
        assert!(info.outline.contains("src/server.ts"));
        assert!(!info.outline.contains("node_modules"));
    }

    #[test]
    fn missing_and_file_paths_are_reported() {
        let missing = inspect("/definitely/not/here/sottovoce");
        assert!(!missing.exists && !missing.is_dir);

        let file = std::env::temp_dir().join("sv-codebase-file.txt");
        fs::write(&file, "x").unwrap();
        let info = inspect(file.to_str().unwrap());
        assert!(info.exists && !info.is_dir);
    }

    #[test]
    fn excerpts_pick_the_relevant_file_and_line() {
        let dir = project("excerpts");
        let base = Codebase::open(dir.to_str().unwrap()).unwrap();
        let block = base
            .context_block(&["createServer".into(), "src/server.ts".into()])
            .unwrap();
        assert!(block.contains("CODEBASE"));
        assert!(block.contains("src/server.ts:1"));
        assert!(block.contains("export function createServer"));
        assert!(!block.contains("ignored()"));

        // Solo il nome del file: si mostra comunque l'inizio per orientarsi.
        let by_name = base.context_block(&["old.rs".into()]).unwrap();
        assert!(by_name.contains("src/old.rs:1"), "{by_name}");
        assert!(by_name.contains("fn legacy_helper"), "{by_name}");
    }

    #[test]
    fn terms_prefer_identifiers_and_skip_filler() {
        let terms = extract_terms(
            [
                "Dobbiamo verificare createServer e il file src/server.ts, poi Redis.",
                "Secondo me è già a posto così.",
            ],
            6,
        );
        assert!(terms.contains(&"createServer".to_string()));
        assert!(terms.contains(&"src/server.ts".to_string()));
        assert!(terms.contains(&"Redis".to_string()));
        assert!(!terms.contains(&"Dobbiamo".to_string()), "{terms:?}");
        assert!(!terms.contains(&"Secondo".to_string()), "{terms:?}");
    }

    #[test]
    fn home_expansion() {
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(expand_home("~/x"), PathBuf::from(home).join("x"));
        }
        assert_eq!(expand_home("/tmp/x"), PathBuf::from("/tmp/x"));
    }
}
