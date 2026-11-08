//! Prova da terminale: parlanti delle frasi di un WAV (16 kHz mono).
//!
//! cargo run --release -p sv-diar --example speakers -- <file.wav> [--speakers N]
//!
//! Frasi dal VAD (Silero), impronta CAM++, poi tracker (automatico) o
//! k-means (`--speakers N`). Modelli dalla cartella dell'app
//! (`cargo run -p sv-models --example fetch -- silero-vad campplus-zh-en`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sv_audio::{AudioFrame, Channel, TARGET_SAMPLE_RATE};
use sv_diar::{CampPlus, Diarizer};
use sv_vad::{Segmenter, VadConfig, VadEvent};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(wav) = args
        .iter()
        .find(|a| !a.starts_with("--") && a.ends_with(".wav"))
    else {
        eprintln!("uso: speakers <file.wav> [--speakers N]");
        std::process::exit(2);
    };
    let speakers: u8 = args
        .iter()
        .position(|a| a == "--speakers")
        .and_then(|i| args.get(i + 1)?.parse().ok())
        .unwrap_or(0);
    let models = PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Application Support/com.giuseppeconcas.sottovoce/models");

    let mut reader = hound::WavReader::open(wav).expect("WAV leggibile");
    let spec = reader.spec();
    assert!(
        spec.sample_rate == TARGET_SAMPLE_RATE && spec.channels == 1,
        "serve un WAV 16 kHz mono (ffmpeg -i in -ar 16000 -ac 1 out.wav)"
    );
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i32>()
            .map(|s| s.unwrap() as f32 / (1u32 << (spec.bits_per_sample - 1)) as f32)
            .collect(),
        hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
    };

    let mut segmenter = Segmenter::new(
        &VadConfig::new(models.join("silero-vad/silero_vad.onnx")),
        Channel::Them,
    )
    .expect("Silero VAD");
    let embedder = CampPlus::load(
        &models.join("campplus-zh-en/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx"),
        1,
    )
    .expect("CAM++");
    let mut diarizer = Diarizer::new(embedder, speakers);

    let mut segments = Vec::new();
    for (i, chunk) in samples.chunks(1024).enumerate() {
        let frame = AudioFrame {
            channel: Channel::Them,
            start: Duration::from_micros((i * 1024) as u64 * 1_000_000 / 16_000),
            samples: Arc::from(chunk),
        };
        segments.extend(segmenter.push(&frame));
    }
    segments.extend(segmenter.flush());

    let mut id = 0;
    for event in segments {
        let VadEvent::Segment(seg) = event else {
            continue;
        };
        id += 1;
        let started = std::time::Instant::now();
        let a = diarizer.assign(id, &seg.samples);
        let who = a
            .speaker
            .map_or_else(|| "?".to_owned(), |s| format!("Speaker {}", s + 1));
        println!(
            "#{id:<3} {:7.2}–{:7.2}s  {who:<10} ({} ms){}",
            seg.start.as_secs_f32(),
            seg.end().as_secs_f32(),
            started.elapsed().as_millis(),
            if a.relabels.is_empty() {
                String::new()
            } else {
                format!("  rietichettate: {:?}", a.relabels)
            }
        );
    }
}
