//! Prova da terminale: stampa le frasi rilevate dal VAD sul microfono (o
//! sull'audio di sistema con `--system`).
//!
//! cargo run -p sv-vad --example segments -- [secondi] [--system]
//!
//! Modello: `SILERO_VAD_MODEL`, altrimenti la cartella modelli dell'app
//! (`cargo run -p sv-models --example fetch -- silero-vad`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use sv_audio::{Capture, CaptureConfig, Channel};
use sv_vad::{Segmenter, VadConfig, VadEvent};
use tokio::sync::broadcast;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let secs: u64 = args.iter().find_map(|a| a.parse().ok()).unwrap_or(30);
    let system = args.iter().any(|a| a == "--system");
    let channel = if system { Channel::Them } else { Channel::Me };
    let model = std::env::var_os("SILERO_VAD_MODEL").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join("Library/Application Support/com.giuseppeconcas.sottovoce/models/silero-vad/silero_vad.onnx")
    });

    let mut segmenter = Segmenter::new(&VadConfig::new(model), channel).unwrap_or_else(|e| {
        eprintln!("{e}\nScaricalo con: cargo run -p sv-models --example fetch -- silero-vad");
        std::process::exit(1);
    });
    let (tx, mut rx) = broadcast::channel(256);
    let capture = Capture::start(
        CaptureConfig {
            mic_device_id: None,
            system_audio: system,
        },
        tx,
        |_| {},
    )
    .expect("avvio cattura");
    println!("Ascolto {channel:?} per {secs} s: parla con delle pause…");

    let deadline = Instant::now() + Duration::from_secs(secs);
    let print = |events: Vec<VadEvent>| {
        for e in events {
            match e {
                VadEvent::SpeechStarted { at, .. } => {
                    println!("{:7.2}s  ▶ parlato", at.as_secs_f32())
                }
                VadEvent::Segment(s) => println!(
                    "{:7.2}s  ■ frase {:.2}–{:.2}s ({:.2}s)",
                    s.end().as_secs_f32(),
                    s.start.as_secs_f32(),
                    s.end().as_secs_f32(),
                    s.duration().as_secs_f32()
                ),
            }
        }
    };
    while Instant::now() < deadline {
        match rx.try_recv() {
            Ok(frame) => print(segmenter.push(&frame)),
            Err(broadcast::error::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(broadcast::error::TryRecvError::Lagged(n)) => eprintln!("persi {n} frame"),
            Err(broadcast::error::TryRecvError::Closed) => break,
        }
    }
    capture.stop();
    print(segmenter.flush());
}
