//! File di testo con il contenuto delle trascrizioni.
//!
//! Il database conserva solo i riferimenti (percorso del file + numero di
//! riga): una riga del file per ogni frase finale e ogni traduzione,
//! nell'ordine in cui sono arrivate. Ogni voce è costretta su una sola riga,
//! così il numero di riga resta un riferimento stabile.
//!
//! I file vivono in `<cartella del DB>/transcripts/<meeting_id>.txt`: sono
//! testo semplice, leggibili e indipendenti da SQLite (esportabili a mano).

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::StoreError;

/// Cartella (relativa al database) dei file di trascrizione.
pub(crate) const DIR: &str = "transcripts";

/// Percorso del file di un meeting, relativo alla cartella del database.
pub(crate) fn relative_path(meeting_id: i64) -> PathBuf {
    Path::new(DIR).join(format!("{meeting_id}.txt"))
}

/// Normalizza il testo su una sola riga: gli a capo diventano spazi.
pub(crate) fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .collect()
}

/// Righe del file; un file assente equivale a nessuna riga.
pub(crate) fn read_lines(path: &Path) -> Result<Vec<String>, StoreError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.lines().map(str::to_owned).collect())
}

fn line_offsets(bytes: &[u8]) -> Vec<u64> {
    let mut offsets = Vec::new();
    let mut start = 0u64;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            offsets.push(start);
            start = index as u64 + 1;
        }
    }
    // Ultima riga senza '\n' (crash o scrittura interrotta).
    if start < bytes.len() as u64 {
        offsets.push(start);
    }
    offsets
}

/// File in scrittura per il thread di salvataggio.
pub(crate) struct TranscriptFile {
    path: PathBuf,
    file: File,
    /// Offset di inizio di ogni riga già presente (0-based).
    offsets: Vec<u64>,
}

impl TranscriptFile {
    /// Apre (creandolo) il file, senza perdere le righe già scritte.
    pub(crate) fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let offsets = line_offsets(&bytes);
        // Un crash può aver lasciato l'ultima riga senza '\n': la chiudiamo
        // prima della prossima append, senza alterare il numero di righe.
        if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
            file.seek(SeekFrom::End(0))?;
            file.write_all(b"\n")?;
        }
        file.seek(SeekFrom::End(0))?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            offsets,
        })
    }

    /// Crea (o svuota) il file. Usato dalla migrazione, che riscrive tutto.
    pub(crate) fn create(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            offsets: Vec::new(),
        })
    }

    /// Numero della prossima riga (1-based).
    fn next_line(&self) -> u64 {
        self.offsets.len() as u64 + 1
    }

    /// Aggiunge una riga e ritorna il suo numero (1-based).
    fn append(&mut self, text: &str) -> Result<u64, StoreError> {
        let line = self.next_line();
        let start = self.file.stream_position()?;
        self.file.write_all(one_line(text).as_bytes())?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        self.offsets.push(start);
        Ok(line)
    }

    /// Salva il testo di una voce: riscrive `line` se esiste, altrimenti
    /// aggiunge una riga nuova. Ritorna il numero di riga effettivo.
    pub(crate) fn save(&mut self, line: Option<u64>, text: &str) -> Result<u64, StoreError> {
        let index = line
            .filter(|line| *line > 0)
            .and_then(|line| usize::try_from(line - 1).ok());
        let Some(index) = index else {
            return self.append(text);
        };
        let mut lines = read_lines(&self.path)?;
        if index >= lines.len() {
            return self.append(text);
        }
        lines[index] = one_line(text);
        let mut content = lines.join("\n");
        content.push('\n');
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(content.as_bytes())?;
        self.file.flush()?;
        self.file.seek(SeekFrom::End(0))?;
        self.offsets = line_offsets(content.as_bytes());
        Ok(index as u64 + 1)
    }
}
