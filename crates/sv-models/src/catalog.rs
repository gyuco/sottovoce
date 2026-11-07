//! Modelli noti all'app. File singoli scaricati da Hugging Face, fissati a un
//! commit: lo SHA-256 atteso non può cambiare sotto i piedi.

use std::sync::LazyLock;

use serde::Serialize;

/// A cosa serve un modello.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelKind {
    /// Rilevamento del parlato (VAD).
    Vad,
    /// Trascrizione locale.
    Stt,
    /// Embedding del parlante (diarization).
    SpeakerEmbedding,
}

/// Un file di un modello.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFile {
    /// Nome del file su disco, dentro la cartella del modello.
    pub name: String,
    #[serde(skip)]
    pub url: String,
    /// Dimensione attesa in byte.
    pub size: u64,
    /// SHA-256 atteso, esadecimale minuscolo.
    #[serde(skip)]
    pub sha256: String,
}

/// Un modello: uno o più file scaricati nella stessa cartella.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpec {
    /// Identificativo stabile (nome della cartella su disco).
    pub id: String,
    pub kind: ModelKind,
    /// Nome leggibile.
    pub name: String,
    pub files: Vec<ModelFile>,
}

impl ModelSpec {
    /// Dimensione totale attesa in byte.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// Silero VAD (incluso nella pipeline locale e nella diarization).
pub const SILERO_VAD: &str = "silero-vad";
/// CAM++ 3D-Speaker zh/en, embedding del parlante (diarization di "Loro").
pub const CAMPLUS: &str = "campplus-zh-en";
/// NVIDIA Parakeet TDT 0.6B v3 int8 (25 lingue europee, italiano incluso).
pub const PARAKEET_V3: &str = "parakeet-tdt-0.6b-v3-int8";

fn hf(repo: &str, commit: &str, name: &str, size: u64, sha256: &str) -> ModelFile {
    ModelFile {
        name: name.to_owned(),
        url: format!("https://huggingface.co/{repo}/resolve/{commit}/{name}"),
        size,
        sha256: sha256.to_owned(),
    }
}

static CATALOG: LazyLock<Vec<ModelSpec>> = LazyLock::new(|| {
    const VAD_REPO: &str = "csukuangfj/vad";
    const VAD_COMMIT: &str = "fba88cd2e921609e7675c3aaf51e0b9b295da4bc";
    const SPK_REPO: &str = "csukuangfj/speaker-embedding-models";
    const SPK_COMMIT: &str = "0743f301363dec56491a490f6d6cbc9d67f9a3bf";
    const PARAKEET_REPO: &str = "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8";
    const PARAKEET_COMMIT: &str = "ed5d745623472f97cfc2e6c2916ee248798fc5a8";

    vec![
        ModelSpec {
            id: SILERO_VAD.into(),
            kind: ModelKind::Vad,
            name: "Silero VAD".into(),
            files: vec![hf(
                VAD_REPO,
                VAD_COMMIT,
                "silero_vad.onnx",
                1_807_522,
                "a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28",
            )],
        },
        ModelSpec {
            id: PARAKEET_V3.into(),
            kind: ModelKind::Stt,
            name: "Parakeet TDT 0.6B v3 (int8)".into(),
            files: vec![
                hf(
                    PARAKEET_REPO,
                    PARAKEET_COMMIT,
                    "encoder.int8.onnx",
                    652_282_294,
                    "1afbe69c1ffdabee94755f42e4daf1d93549131a3da11356bda84fcef0f2c479",
                ),
                hf(
                    PARAKEET_REPO,
                    PARAKEET_COMMIT,
                    "decoder.int8.onnx",
                    11_845_274,
                    "0222a86147299d4dee90edf95b6d13827edb42ad8b7c9cf0b2582f030937c0b5",
                ),
                hf(
                    PARAKEET_REPO,
                    PARAKEET_COMMIT,
                    "joiner.int8.onnx",
                    6_355_277,
                    "cbd02f3696e9944970111fb439d9c3fd365a80374565f13f2b9c7007d253fc98",
                ),
                hf(
                    PARAKEET_REPO,
                    PARAKEET_COMMIT,
                    "tokens.txt",
                    93_939,
                    "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
                ),
            ],
        },
        ModelSpec {
            id: CAMPLUS.into(),
            kind: ModelKind::SpeakerEmbedding,
            name: "CAM++ 3D-Speaker zh/en".into(),
            files: vec![hf(
                SPK_REPO,
                SPK_COMMIT,
                "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx",
                28_281_164,
                "aa3cfc16963a10586a9393f5035d6d6b57e98d358b347f80c2a30bf4f00ceba2",
            )],
        },
    ]
});

/// Tutti i modelli noti.
pub fn catalog() -> &'static [ModelSpec] {
    &CATALOG
}

/// Modello per id.
pub fn find(id: &str) -> Option<&'static ModelSpec> {
    CATALOG.iter().find(|m| m.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_well_formed() {
        let mut ids: Vec<_> = catalog().iter().map(|m| m.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), catalog().len(), "id duplicati");
        for m in catalog() {
            assert!(!m.files.is_empty(), "{}", m.id);
            for f in &m.files {
                assert_eq!(f.sha256.len(), 64, "{}/{}", m.id, f.name);
                assert!(
                    f.sha256
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                );
                assert!(f.url.starts_with("https://") && f.url.ends_with(&f.name));
                assert!(f.size > 0 && !f.name.contains('/'));
            }
        }
        for id in [SILERO_VAD, CAMPLUS, PARAKEET_V3] {
            assert!(find(id).is_some(), "{id}");
        }
    }
}
