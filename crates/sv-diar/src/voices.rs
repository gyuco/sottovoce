//! Profili vocali: riconoscere gli stessi parlanti tra meeting diversi.
//!
//! Funzione opt-in: un'impronta vocale è un dato biometrico. Si salva solo il
//! profilo di un parlante a cui l'utente ha dato un nome, mai profili
//! anonimi, e tutto resta in locale.
//!
//! - Durante la sessione il [`Recognizer`] confronta il centroide di ogni
//!   parlante (media delle impronte delle sue frasi, pesata per durata) con i
//!   profili noti: sopra [`MatchThresholds::assign`] il nome è assegnato,
//!   tra `suggest` e `assign` viene solo suggerito.
//! - A fine sessione [`VoiceStore::save_session`] aggiorna (o crea) il
//!   profilo di ogni parlante con nome che ha parlato abbastanza.
//!
//! Su disco, in una cartella dedicata (`voices/`): un file `<id>.bin` per
//! profilo (centroide f32 + secondi di parlato) e `profiles.json` con nome e
//! date. Nessun dato nel database dei meeting.

use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Speaker;
use crate::tracker::{dot, normalized};

/// Parlato minimo (s) in un meeting per salvare o aggiornare un profilo.
pub const MIN_PROFILE_S: f32 = 20.0;
/// Peso massimo (s) del profilo esistente nella media: il profilo resta
/// capace di adattarsi alla voce (microfono, rete) dopo molti meeting.
const MAX_WEIGHT_S: f32 = 600.0;
/// Frasi più corte non entrano nel centroide del parlante: impronta instabile.
pub const MIN_SEGMENT_S: f32 = 1.0;

const INDEX_FILE: &str = "profiles.json";
const MAGIC: &[u8; 4] = b"SVVP";
const VERSION: u16 = 1;
const MAX_NAME_CHARS: usize = 60;

/// Soglie del riconoscimento (similarità coseno tra centroidi).
///
/// Più severe di quella interna al meeting (0.45 tra singola frase e
/// profilo): qui un errore attribuisce il nome sbagliato a una persona.
/// Taratura (`examples/voices.rs`, voci sintetiche di macOS, ~20 s per
/// meeting): stessa voce ≈ 0.98, voci diverse 0.12–0.55, varianti dello
/// stesso sintetizzatore 0.68–0.74. Da rivedere con registrazioni reali.
#[derive(Debug, Clone, Copy)]
pub struct MatchThresholds {
    /// Sopra: nome assegnato.
    pub assign: f32,
    /// Sopra (e sotto `assign`): nome suggerito, da confermare.
    pub suggest: f32,
    /// Parlato (s) del parlante nella sessione prima di provarci.
    pub min_speech_s: f32,
}

impl Default for MatchThresholds {
    fn default() -> Self {
        Self {
            assign: 0.80,
            suggest: 0.65,
            min_speech_s: 6.0,
        }
    }
}

/// Profilo noto caricato per la sessione.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownVoice {
    pub id: String,
    pub centroid: Vec<f32>,
}

/// Voce di un parlante nella sessione corrente.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerVoice {
    pub speaker: Speaker,
    /// Normalizzato.
    pub centroid: Vec<f32>,
    /// Parlato (s) che ha contribuito al centroide.
    pub seconds: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Assign,
    Suggest,
}

/// Un parlante della sessione riconosciuto in un profilo.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceMatch {
    pub speaker: Speaker,
    pub profile: String,
    pub similarity: f32,
    pub verdict: Verdict,
}

/// Accoppia parlanti e profili sopra `suggest`: prima le coppie più simili,
/// ogni parlante e ogni profilo al massimo una volta. Sotto
/// `min_speech_s` di parlato il parlante non viene considerato.
pub fn match_voices(
    speakers: &[SpeakerVoice],
    known: &[KnownVoice],
    th: &MatchThresholds,
) -> Vec<VoiceMatch> {
    let mut pairs: Vec<(usize, usize, f32)> = speakers
        .iter()
        .enumerate()
        .filter(|(_, s)| s.seconds >= th.min_speech_s)
        .flat_map(|(i, s)| {
            known
                .iter()
                .enumerate()
                .map(move |(j, k)| (i, j, dot(&s.centroid, &k.centroid)))
        })
        .filter(|(_, _, sim)| *sim >= th.suggest)
        .collect();
    pairs.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut used_speaker = vec![false; speakers.len()];
    let mut used_profile = vec![false; known.len()];
    let mut out = Vec::new();
    for (i, j, sim) in pairs {
        if used_speaker[i] || used_profile[j] {
            continue;
        }
        used_speaker[i] = true;
        used_profile[j] = true;
        out.push(VoiceMatch {
            speaker: speakers[i].speaker,
            profile: known[j].id.clone(),
            similarity: sim,
            verdict: if sim >= th.assign {
                Verdict::Assign
            } else {
                Verdict::Suggest
            },
        });
    }
    out
}

/// Riconoscimento durante la sessione: segnala solo le novità (nuovo
/// suggerimento, suggerimento cambiato, suggerimento diventato
/// assegnazione). Un'assegnazione è definitiva per la sessione: parlante e
/// profilo escono dai confronti successivi.
pub struct Recognizer {
    known: Vec<KnownVoice>,
    thresholds: MatchThresholds,
    reported: HashMap<Speaker, (String, Verdict)>,
}

impl Recognizer {
    pub fn new(known: Vec<KnownVoice>, thresholds: MatchThresholds) -> Self {
        let known = known
            .into_iter()
            .filter_map(|k| {
                normalized(&k.centroid).map(|centroid| KnownVoice { id: k.id, centroid })
            })
            .collect();
        Self {
            known,
            thresholds,
            reported: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.known.is_empty()
    }

    pub fn update(&mut self, voices: &[SpeakerVoice]) -> Vec<VoiceMatch> {
        let assigned = |r: &HashMap<Speaker, (String, Verdict)>| -> Vec<(Speaker, String)> {
            r.iter()
                .filter(|(_, (_, v))| *v == Verdict::Assign)
                .map(|(s, (p, _))| (*s, p.clone()))
                .collect()
        };
        let frozen = assigned(&self.reported);
        let speakers: Vec<SpeakerVoice> = voices
            .iter()
            .filter(|v| !frozen.iter().any(|(s, _)| *s == v.speaker))
            .cloned()
            .collect();
        let known: Vec<KnownVoice> = self
            .known
            .iter()
            .filter(|k| !frozen.iter().any(|(_, p)| *p == k.id))
            .cloned()
            .collect();
        let mut changes = Vec::new();
        for m in match_voices(&speakers, &known, &self.thresholds) {
            let current = (m.profile.clone(), m.verdict);
            if self.reported.get(&m.speaker) != Some(&current) {
                self.reported.insert(m.speaker, current);
                changes.push(m);
            }
        }
        changes
    }
}

/// Media pesata di due centroidi, normalizzata. Il peso del vecchio è
/// limitato a [`MAX_WEIGHT_S`].
pub fn merge_centroid(old: &[f32], old_s: f32, new: &[f32], new_s: f32) -> Vec<f32> {
    let w_old = old_s.clamp(0.0, MAX_WEIGHT_S);
    let w_new = new_s.max(0.0);
    let mixed: Vec<f32> = old
        .iter()
        .zip(new)
        .map(|(a, b)| a * w_old + b * w_new)
        .collect();
    normalized(&mixed).unwrap_or_else(|| new.to_vec())
}

/// Metadati di un profilo (senza impronta).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProfile {
    pub id: String,
    pub name: String,
    pub created_ms: u64,
    /// Ultimo meeting in cui il profilo è stato aggiornato.
    pub last_seen_ms: u64,
    pub meetings: u32,
}

/// Impronta salvata in `<id>.bin`.
#[derive(Debug, Clone, PartialEq)]
pub struct VoicePrint {
    pub centroid: Vec<f32>,
    pub seconds: f32,
}

impl VoicePrint {
    pub fn to_bytes(&self) -> Vec<u8> {
        let dim = u16::try_from(self.centroid.len()).unwrap_or(u16::MAX);
        let mut out = Vec::with_capacity(12 + 4 * usize::from(dim));
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&dim.to_le_bytes());
        out.extend_from_slice(&self.seconds.to_le_bytes());
        for x in self.centroid.iter().take(usize::from(dim)) {
            out.extend_from_slice(&x.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let header = bytes.get(..12)?;
        if &header[..4] != MAGIC || u16::from_le_bytes([header[4], header[5]]) != VERSION {
            return None;
        }
        let dim = usize::from(u16::from_le_bytes([header[6], header[7]]));
        let seconds = f32::from_le_bytes(header[8..12].try_into().ok()?);
        let body = bytes.get(12..)?;
        if body.len() != dim * 4 || dim == 0 || !seconds.is_finite() {
            return None;
        }
        let centroid: Vec<f32> = body
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        centroid
            .iter()
            .all(|x| x.is_finite())
            .then_some(Self { centroid, seconds })
    }
}

/// Esito del salvataggio di un parlante a fine sessione.
#[derive(Debug, Clone, PartialEq)]
pub enum Saved {
    Created(String),
    Updated(String),
    /// Troppo poco parlato.
    TooShort,
    /// Il profilo con lo stesso nome ha una voce troppo diversa: non lo si
    /// sporca con un'altra persona.
    Mismatch(String),
}

/// Cartella dei profili. Non crea nulla finché non si salva un profilo.
#[derive(Debug, Clone)]
pub struct VoiceStore {
    dir: PathBuf,
}

impl VoiceStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Profili ordinati per ultimo meeting (più recente prima).
    pub fn list(&self) -> io::Result<Vec<VoiceProfile>> {
        let mut list = self.read_index()?;
        list.sort_by(|a, b| b.last_seen_ms.cmp(&a.last_seen_ms));
        Ok(list)
    }

    /// Profili con impronta leggibile, con il loro nome.
    pub fn known(&self) -> io::Result<Vec<(KnownVoice, String)>> {
        let mut out = Vec::new();
        for p in self.read_index()? {
            match self.read_print(&p.id) {
                Ok(print) => out.push((
                    KnownVoice {
                        id: p.id,
                        centroid: print.centroid,
                    },
                    p.name,
                )),
                Err(e) => tracing::warn!(profile = %p.id, "profilo vocale illeggibile: {e}"),
            }
        }
        Ok(out)
    }

    /// Aggiorna o crea i profili dei parlanti con nome. Il profilo con lo
    /// stesso nome (senza distinzione di maiuscole) viene aggiornato solo se
    /// la voce gli somiglia almeno quanto `th.suggest`.
    pub fn save_session(
        &self,
        named: &[(String, SpeakerVoice)],
        th: &MatchThresholds,
        now_ms: u64,
    ) -> io::Result<Vec<(String, Saved)>> {
        let mut index = self.read_index()?;
        let mut out = Vec::new();
        for (name, voice) in named {
            let name = clip_name(name);
            if name.is_empty() {
                continue;
            }
            if voice.seconds < MIN_PROFILE_S {
                out.push((name, Saved::TooShort));
                continue;
            }
            let Some(centroid) = normalized(&voice.centroid) else {
                continue;
            };
            let key = name.to_lowercase();
            let existing = index.iter_mut().find(|p| p.name.to_lowercase() == key);
            let saved = match existing {
                Some(profile) => match self.read_print(&profile.id) {
                    Ok(old) if dot(&old.centroid, &centroid) < th.suggest => {
                        Saved::Mismatch(profile.id.clone())
                    }
                    Ok(old) => {
                        let print = VoicePrint {
                            centroid: merge_centroid(
                                &old.centroid,
                                old.seconds,
                                &centroid,
                                voice.seconds,
                            ),
                            seconds: old.seconds + voice.seconds,
                        };
                        self.write_print(&profile.id, &print)?;
                        profile.last_seen_ms = now_ms;
                        profile.meetings = profile.meetings.saturating_add(1);
                        Saved::Updated(profile.id.clone())
                    }
                    // Impronta persa o rovinata: la si riscrive da questa sessione.
                    Err(_) => {
                        self.write_print(
                            &profile.id,
                            &VoicePrint {
                                centroid,
                                seconds: voice.seconds,
                            },
                        )?;
                        profile.last_seen_ms = now_ms;
                        profile.meetings = profile.meetings.saturating_add(1);
                        Saved::Updated(profile.id.clone())
                    }
                },
                None => {
                    let id = new_id(&index, now_ms);
                    self.write_print(
                        &id,
                        &VoicePrint {
                            centroid,
                            seconds: voice.seconds,
                        },
                    )?;
                    index.push(VoiceProfile {
                        id: id.clone(),
                        name: name.clone(),
                        created_ms: now_ms,
                        last_seen_ms: now_ms,
                        meetings: 1,
                    });
                    Saved::Created(id)
                }
            };
            out.push((name, saved));
        }
        if out
            .iter()
            .any(|(_, s)| matches!(s, Saved::Created(_) | Saved::Updated(_)))
        {
            self.write_index(&index)?;
        }
        Ok(out)
    }

    pub fn rename(&self, id: &str, name: &str) -> io::Result<VoiceProfile> {
        check_id(id)?;
        let name = clip_name(name);
        if name.is_empty() {
            return Err(io::Error::new(ErrorKind::InvalidInput, "nome vuoto"));
        }
        let mut index = self.read_index()?;
        let key = name.to_lowercase();
        if index
            .iter()
            .any(|p| p.id != id && p.name.to_lowercase() == key)
        {
            return Err(io::Error::new(
                ErrorKind::AlreadyExists,
                "esiste già un profilo con questo nome",
            ));
        }
        let profile = index
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "profilo non trovato"))?;
        profile.name = name;
        let renamed = profile.clone();
        self.write_index(&index)?;
        Ok(renamed)
    }

    /// Elimina profilo e impronta. Senza più profili la cartella sparisce.
    pub fn delete(&self, id: &str) -> io::Result<()> {
        check_id(id)?;
        let mut index = self.read_index()?;
        index.retain(|p| p.id != id);
        remove_if_exists(&self.print_path(id))?;
        if index.is_empty() {
            return self.delete_all();
        }
        self.write_index(&index)
    }

    /// Elimina tutti i profili (l'intera cartella).
    pub fn delete_all(&self) -> io::Result<()> {
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join(INDEX_FILE)
    }

    fn print_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.bin"))
    }

    fn read_index(&self) -> io::Result<Vec<VoiceProfile>> {
        match std::fs::read(self.index_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    fn write_index(&self, index: &[VoiceProfile]) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(index)
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
        self.write_private(&self.index_path(), &bytes)
    }

    fn read_print(&self, id: &str) -> io::Result<VoicePrint> {
        check_id(id)?;
        VoicePrint::from_bytes(&std::fs::read(self.print_path(id))?)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "impronta non valida"))
    }

    fn write_print(&self, id: &str, print: &VoicePrint) -> io::Result<()> {
        check_id(id)?;
        self.write_private(&self.print_path(id), &print.to_bytes())
    }

    /// Scrittura atomica, leggibile solo dall'utente.
    fn write_private(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, path)
    }
}

fn clip_name(name: &str) -> String {
    name.trim().chars().take(MAX_NAME_CHARS).collect()
}

/// Gli id arrivano anche dal frontend: solo `[a-z0-9]`, niente percorsi.
fn check_id(id: &str) -> io::Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase());
    if ok {
        Ok(())
    } else {
        Err(io::Error::new(
            ErrorKind::InvalidInput,
            "id profilo non valido",
        ))
    }
}

fn new_id(index: &[VoiceProfile], now_ms: u64) -> String {
    (0u32..)
        .map(|n| format!("v{now_ms:x}{n:x}"))
        .find(|id| index.iter().all(|p| p.id != *id))
        .expect("id libero")
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::voice;

    fn sv(speaker: Speaker, axis: usize, seconds: f32, seed: u32) -> SpeakerVoice {
        SpeakerVoice {
            speaker,
            centroid: voice(axis, 0.2, seed),
            seconds,
        }
    }

    fn kv(id: &str, axis: usize, seed: u32) -> KnownVoice {
        KnownVoice {
            id: id.into(),
            centroid: voice(axis, 0.2, seed),
        }
    }

    /// Vettore unitario a similarità `sim` con l'asse 0.
    fn at_similarity(sim: f32) -> Vec<f32> {
        let mut v = vec![0.0; 16];
        v[0] = sim;
        v[1] = (1.0 - sim * sim).sqrt();
        v
    }

    fn axis(i: usize) -> Vec<f32> {
        let mut v = vec![0.0; 16];
        v[i] = 1.0;
        v
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sv-diar-voices-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn thresholds_assign_suggest_ignore() {
        let th = MatchThresholds::default();
        let known = [KnownVoice {
            id: "marco".into(),
            centroid: axis(0),
        }];
        let at = |sim: f32| {
            match_voices(
                &[SpeakerVoice {
                    speaker: 0,
                    centroid: at_similarity(sim),
                    seconds: 10.0,
                }],
                &known,
                &th,
            )
        };
        assert_eq!(at(0.9)[0].verdict, Verdict::Assign);
        assert_eq!(at(th.assign)[0].verdict, Verdict::Assign);
        assert_eq!(at(0.72)[0].verdict, Verdict::Suggest);
        assert!(at(0.55).is_empty());
        // Poco parlato: nessun confronto.
        let short = SpeakerVoice {
            speaker: 0,
            centroid: axis(0),
            seconds: th.min_speech_s - 0.1,
        };
        assert!(match_voices(&[short], &known, &th).is_empty());
    }

    #[test]
    fn matching_is_one_to_one() {
        let th = MatchThresholds::default();
        let speakers = [sv(0, 1, 10.0, 1), sv(1, 0, 10.0, 2), sv(2, 5, 10.0, 3)];
        let known = [kv("anna", 0, 4), kv("marco", 1, 5), kv("luca", 9, 6)];
        let mut got: Vec<_> = match_voices(&speakers, &known, &th)
            .into_iter()
            .map(|m| (m.speaker, m.profile, m.verdict))
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                (0, "marco".to_owned(), Verdict::Assign),
                (1, "anna".to_owned(), Verdict::Assign),
            ]
        );
    }

    #[test]
    fn recognizer_reports_changes_once_and_freezes_assignments() {
        let th = MatchThresholds::default();
        let mut r = Recognizer::new(
            vec![KnownVoice {
                id: "marco".into(),
                centroid: axis(0),
            }],
            th,
        );
        let voice_at = |sim: f32, s: Speaker| SpeakerVoice {
            speaker: s,
            centroid: at_similarity(sim),
            seconds: 10.0,
        };
        let first = r.update(&[voice_at(0.7, 0)]);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].verdict, Verdict::Suggest);
        // Stesso esito: niente da segnalare.
        assert!(r.update(&[voice_at(0.72, 0)]).is_empty());
        // Il suggerimento diventa assegnazione.
        let up = r.update(&[voice_at(0.9, 0)]);
        assert_eq!(up[0].verdict, Verdict::Assign);
        // Assegnato: un altro parlante simile non prende lo stesso profilo.
        assert!(r.update(&[voice_at(0.4, 0), voice_at(0.95, 1)]).is_empty());
    }

    #[test]
    fn merge_weights_and_caps_old_profile() {
        let a = axis(0);
        let b = axis(1);
        let m = merge_centroid(&a, 30.0, &b, 30.0);
        assert!((m[0] - m[1]).abs() < 1e-6);
        assert!((dot(&m, &m) - 1.0).abs() < 1e-5);
        // Profilo molto vecchio: pesa al massimo MAX_WEIGHT_S.
        let capped = merge_centroid(&a, 100_000.0, &b, MAX_WEIGHT_S);
        assert!((capped[0] - capped[1]).abs() < 1e-6);
        let light = merge_centroid(&a, 60.0, &b, 20.0);
        assert!(light[0] > light[1]);
    }

    #[test]
    fn print_round_trip_and_rejects_garbage() {
        let p = VoicePrint {
            centroid: voice(3, 0.2, 7),
            seconds: 42.5,
        };
        assert_eq!(VoicePrint::from_bytes(&p.to_bytes()), Some(p.clone()));
        let mut bad = p.to_bytes();
        bad[0] = b'X';
        assert_eq!(VoicePrint::from_bytes(&bad), None);
        assert_eq!(VoicePrint::from_bytes(&p.to_bytes()[..20]), None);
        assert_eq!(VoicePrint::from_bytes(&[]), None);
    }

    #[test]
    fn store_creates_updates_and_deletes() {
        let dir = temp_dir("store");
        let store = VoiceStore::new(&dir);
        let th = MatchThresholds::default();
        assert!(store.list().unwrap().is_empty());
        assert!(!dir.exists());

        // Poco parlato: niente file.
        let short = store
            .save_session(&[("Marco".into(), sv(0, 0, 5.0, 1))], &th, 1_000)
            .unwrap();
        assert_eq!(short[0].1, Saved::TooShort);
        assert!(!dir.exists());

        let saved = store
            .save_session(
                &[
                    ("Marco".into(), sv(0, 0, 30.0, 1)),
                    ("  ".into(), sv(1, 1, 30.0, 2)),
                ],
                &th,
                1_000,
            )
            .unwrap();
        assert_eq!(saved.len(), 1);
        let Saved::Created(id) = &saved[0].1 else {
            panic!("atteso Created: {saved:?}")
        };
        assert!(dir.join(format!("{id}.bin")).is_file());

        // Stessa voce, nome con maiuscole diverse: aggiorna.
        let again = store
            .save_session(&[("marco".into(), sv(2, 0, 25.0, 3))], &th, 2_000)
            .unwrap();
        assert_eq!(again[0].1, Saved::Updated(id.clone()));
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].meetings, 2);
        assert_eq!(list[0].last_seen_ms, 2_000);
        assert_eq!(list[0].name, "Marco");
        let (known, name) = &store.known().unwrap()[0];
        assert_eq!(name, "Marco");
        assert_eq!(store.read_print(&known.id).unwrap().seconds, 55.0);

        // Voce diversa con lo stesso nome: profilo intatto.
        let other = store
            .save_session(&[("Marco".into(), sv(0, 7, 30.0, 4))], &th, 3_000)
            .unwrap();
        assert_eq!(other[0].1, Saved::Mismatch(id.clone()));
        assert_eq!(store.list().unwrap()[0].meetings, 2);

        // Rinomina, nome già usato, id non valido.
        store
            .save_session(&[("Anna".into(), sv(0, 4, 30.0, 5))], &th, 4_000)
            .unwrap();
        assert_eq!(store.rename(id, "Marco R.").unwrap().name, "Marco R.");
        assert!(store.rename(id, "anna").is_err());
        assert!(store.rename("../x", "y").is_err());
        assert!(store.delete("../../etc").is_err());

        store.delete(id).unwrap();
        assert!(!dir.join(format!("{id}.bin")).exists());
        assert_eq!(store.list().unwrap().len(), 1);
        store.delete_all().unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn deleting_last_profile_removes_folder() {
        let dir = temp_dir("last");
        let store = VoiceStore::new(&dir);
        let saved = store
            .save_session(
                &[("Luca".into(), sv(0, 2, 30.0, 1))],
                &MatchThresholds::default(),
                1,
            )
            .unwrap();
        let Saved::Created(id) = &saved[0].1 else {
            panic!()
        };
        store.delete(id).unwrap();
        assert!(!dir.exists());
    }
}
