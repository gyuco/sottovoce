//! Taratura dei profili vocali: similarità tra le voci di più WAV.
//!
//! cargo run --release -p sv-diar --example voices -- <a.wav> <b.wav> ...
//!
//! Ogni WAV (16 kHz mono) = un parlante in un "meeting". Per ciascuno il
//! centroide come in sessione (frasi dal VAD da almeno 1 s, impronte CAM++
//! pesate per durata), poi la matrice delle similarità coseno con l'esito
//! delle soglie di default: A = assegna, S = suggerisci, · = ignora. Due file
//! della stessa voce dovrebbero stare in A (o S), voci diverse in ·.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sv_audio::{AudioFrame, Channel, TARGET_SAMPLE_RATE};
use sv_diar::voices::MIN_SEGMENT_S;
use sv_diar::{CampPlus, Embed, MatchThresholds};
use sv_vad::{Segmenter, VadConfig, VadEvent};

fn main() {
    let wavs: Vec<String> = std::env::args().skip(1).collect();
    if wavs.len() < 2 {
        eprintln!("uso: voices <a.wav> <b.wav> ...");
        std::process::exit(2);
    }
    let models = PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Application Support/com.giuseppeconcas.sottovoce/models");
    let vad = VadConfig::new(models.join("silero-vad/silero_vad.onnx"));
    let mut embedder = CampPlus::load(
        &models.join("campplus-zh-en/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx"),
        1,
    )
    .expect("CAM++");

    let mut voices = Vec::new();
    for wav in &wavs {
        let (centroid, seconds) = centroid(wav, &vad, &mut embedder);
        let name = Path::new(wav)
            .file_stem()
            .map_or_else(|| wav.clone(), |s| s.to_string_lossy().into_owned());
        println!("{name:<16} {seconds:6.1} s di parlato");
        voices.push((name, centroid));
    }

    let th = MatchThresholds::default();
    println!(
        "\nsoglie: assegna ≥ {:.2}, suggerisci ≥ {:.2}\n",
        th.assign, th.suggest
    );
    print!("{:<16}", "");
    for (n, _) in &voices {
        print!(" {:>12.12}", n);
    }
    println!();
    for (a, va) in &voices {
        print!("{a:<16}");
        for (_, vb) in &voices {
            let sim: f32 = va.iter().zip(vb).map(|(x, y)| x * y).sum();
            let mark = if sim >= th.assign {
                'A'
            } else if sim >= th.suggest {
                'S'
            } else {
                '·'
            };
            print!(" {:>10.3} {mark}", sim);
        }
        println!();
    }
}

fn centroid(wav: &str, vad: &VadConfig, embedder: &mut CampPlus) -> (Vec<f32>, f32) {
    let mut reader = hound::WavReader::open(wav).expect("WAV leggibile");
    let spec = reader.spec();
    assert!(
        spec.sample_rate == TARGET_SAMPLE_RATE && spec.channels == 1,
        "{wav}: serve un WAV 16 kHz mono"
    );
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i32>()
            .map(|s| s.unwrap() as f32 / (1u32 << (spec.bits_per_sample - 1)) as f32)
            .collect(),
        hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
    };
    let mut segmenter = Segmenter::new(vad, Channel::Them).expect("Silero VAD");
    let mut events = Vec::new();
    for (i, chunk) in samples.chunks(1024).enumerate() {
        events.extend(segmenter.push(&AudioFrame {
            channel: Channel::Them,
            start: Duration::from_micros((i * 1024) as u64 * 1_000_000 / 16_000),
            samples: Arc::from(chunk),
        }));
    }
    events.extend(segmenter.flush());

    let mut sum: Vec<f32> = Vec::new();
    let mut seconds = 0.0;
    for event in events {
        let VadEvent::Segment(seg) = event else {
            continue;
        };
        let duration = seg.samples.len() as f32 / TARGET_SAMPLE_RATE as f32;
        if duration < MIN_SEGMENT_S {
            continue;
        }
        let Some(v) = embedder.embed(&seg.samples) else {
            continue;
        };
        if sum.is_empty() {
            sum = vec![0.0; v.len()];
        }
        sum.iter_mut().zip(&v).for_each(|(s, x)| *s += x * duration);
        seconds += duration;
    }
    let norm = sum
        .iter()
        .map(|x| x * x)
        .sum::<f32>()
        .sqrt()
        .max(f32::EPSILON);
    (sum.iter().map(|x| x / norm).collect(), seconds)
}
