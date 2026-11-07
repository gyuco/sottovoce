//! Con il modello CAM++ reale (saltato se assente): due frasi della stessa
//! voce → stesso parlante.

use std::path::PathBuf;

use sv_diar::{CampPlus, Diarizer, Embed};

fn model() -> Option<PathBuf> {
    let path = std::env::var_os("CAMPLUS_MODEL").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(
            "Library/Application Support/com.giuseppeconcas.sottovoce/models/campplus-zh-en/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx",
        )
    });
    if path.is_file() {
        Some(path)
    } else {
        eprintln!("SALTATO: modello CAM++ assente ({})", path.display());
        None
    }
}

fn fixture() -> Vec<f32> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../sv-vad/tests/fixtures/two_phrases_it.wav"
    );
    hound::WavReader::open(path)
        .unwrap()
        .samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / 32768.0)
        .collect()
}

#[test]
fn same_voice_same_speaker() {
    let Some(model) = model() else { return };
    let audio = fixture();
    // Prima frase ~0–4 s, seconda da ~5 s (vedi i test di sv-vad).
    let (a, b) = (&audio[..16_000 * 4], &audio[16_000 * 5..]);

    let mut embedder = CampPlus::load(&model, 1).unwrap();
    let (va, vb) = (embedder.embed(a).unwrap(), embedder.embed(b).unwrap());
    let sim: f32 = va.iter().zip(&vb).map(|(x, y)| x * y).sum();
    assert!(sim > 0.45, "similarità {sim}");
    assert!(embedder.embed(&[0.0; 16_000]).is_none(), "silenzio");

    let mut d = Diarizer::new(embedder, 0);
    assert_eq!(d.assign(1, a).speaker, Some(0));
    assert_eq!(d.assign(2, b).speaker, Some(0));
}
