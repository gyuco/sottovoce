//! Test con Silero VAD reale su un WAV (voce sintetica it): due frasi
//! separate da 1,5 s di silenzio. Servono il modello: `SILERO_VAD_MODEL`
//! oppure la cartella modelli dell'app
//! (`cargo run -p sv-models --example fetch -- silero-vad`). Senza modello
//! i test vengono saltati con un avviso.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sv_audio::{AudioFrame, Channel};
use sv_vad::{Segmenter, SpeechSegment, VadConfig, VadEvent};

const FRAME: usize = 1024;
/// Confini attesi della voce nel fixture (s): silenzio 1,0 + 2,68 + 1,5 + 1,73 + 1,0.
const PHRASES: [(f64, f64); 2] = [(1.0, 3.68), (5.18, 6.91)];
const TOLERANCE: f64 = 0.4;

fn model() -> Option<PathBuf> {
    let path = std::env::var_os("SILERO_VAD_MODEL").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join("Library/Application Support/com.giuseppeconcas.sottovoce/models/silero-vad/silero_vad.onnx")
    });
    if path.is_file() {
        Some(path)
    } else {
        eprintln!("SALTATO: modello Silero VAD assente ({})", path.display());
        None
    }
}

fn fixture() -> Vec<f32> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/two_phrases_it.wav"
    );
    let mut reader = hound::WavReader::open(path).unwrap();
    assert_eq!(reader.spec().sample_rate, 16_000);
    reader
        .samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / 32768.0)
        .collect()
}

fn frames(samples: &[f32], channel: Channel) -> impl Iterator<Item = AudioFrame> + '_ {
    samples
        .chunks(FRAME)
        .enumerate()
        .map(move |(i, c)| AudioFrame {
            channel,
            start: Duration::from_micros((i * FRAME) as u64 * 1_000_000 / 16_000),
            samples: Arc::from(c),
        })
}

fn run(seg: &mut Segmenter, samples: &[f32], channel: Channel) -> (Vec<SpeechSegment>, usize) {
    let mut out = Vec::new();
    let mut started = 0;
    let mut collect = |events: Vec<VadEvent>| {
        for e in events {
            match e {
                VadEvent::Segment(s) => out.push(s),
                VadEvent::SpeechStarted { .. } => started += 1,
            }
        }
    };
    for f in frames(samples, channel) {
        collect(seg.push(&f));
    }
    collect(seg.flush());
    (out, started)
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

#[test]
fn splits_two_phrases_with_padding() {
    let Some(model) = model() else { return };
    let config = VadConfig::new(model);
    let mut seg = Segmenter::new(&config, Channel::Them).unwrap();
    let (segments, started) = run(&mut seg, &fixture(), Channel::Them);

    assert_eq!(
        segments.len(),
        2,
        "{:?}",
        segments
            .iter()
            .map(|s| (secs(s.start), secs(s.end())))
            .collect::<Vec<_>>()
    );
    assert_eq!(started, 2);
    let pad = config.padding.as_secs_f64();
    for (s, (from, to)) in segments.iter().zip(PHRASES) {
        assert_eq!(s.channel, Channel::Them);
        assert!(
            (secs(s.start) - (from - pad)).abs() < TOLERANCE,
            "inizio {} atteso ~{}",
            secs(s.start),
            from - pad
        );
        assert!(
            (secs(s.end()) - (to + pad)).abs() < TOLERANCE,
            "fine {} attesa ~{}",
            secs(s.end()),
            to + pad
        );
        assert_eq!(
            s.samples.len() as f64 / 16_000.0,
            secs(s.duration()).max(0.0)
        );
    }
    assert!(
        segments[0].end() <= segments[1].start,
        "padding sovrapposti"
    );
}

#[test]
fn silence_produces_nothing() {
    let Some(model) = model() else { return };
    let mut seg = Segmenter::new(&VadConfig::new(model), Channel::Me).unwrap();
    let (segments, started) = run(&mut seg, &vec![0.0; 16_000 * 5], Channel::Me);
    assert!(segments.is_empty());
    assert_eq!(started, 0);
}

#[test]
fn long_speech_is_cut_at_max_duration() {
    let Some(model) = model() else { return };
    let mut config = VadConfig::new(model);
    config.max_speech = Duration::from_secs(1);
    let mut seg = Segmenter::new(&config, Channel::Me).unwrap();
    let (segments, _) = run(&mut seg, &fixture(), Channel::Me);

    assert!(
        segments.len() >= 4,
        "attese frasi tagliate, trovate {}",
        segments.len()
    );
    let max = config.max_speech + config.padding * 2;
    assert!(
        segments.iter().all(|s| s.duration() <= max),
        "frase oltre il massimo"
    );
    assert!(
        segments.windows(2).all(|w| w[0].end() <= w[1].start),
        "frasi sovrapposte"
    );
}

#[test]
fn flush_closes_speech_in_progress_and_ignores_other_channel() {
    let Some(model) = model() else { return };
    let mut seg = Segmenter::new(&VadConfig::new(model), Channel::Me).unwrap();
    let audio = fixture();

    // Frame dell'altro canale: ignorati.
    assert!(frames(&audio, Channel::Them).all(|f| seg.push(&f).is_empty()));

    // Audio troncato a metà della seconda frase: la chiude il flush.
    let cut = (6.0 * 16_000.0) as usize;
    let (segments, _) = run(&mut seg, &audio[..cut], Channel::Me);
    assert_eq!(segments.len(), 2);
    assert!(secs(segments[1].end()) <= 6.0 + 0.01);
}
