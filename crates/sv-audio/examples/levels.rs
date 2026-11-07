//! Prova da terminale: stampa i livelli di "Io" e "Loro" per qualche secondo.
//!
//! cargo run -p sv-audio --example levels -- [secondi]

use std::time::Duration;

use sv_audio::{Capture, CaptureConfig, Channel, list_input_devices};
use tokio::sync::broadcast;

fn main() {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    for d in list_input_devices().expect("elenco device") {
        println!(
            "{} {} ({})",
            if d.is_default { "*" } else { " " },
            d.name,
            d.id
        );
    }
    let (tx, mut rx) = broadcast::channel(256);
    let capture = Capture::start(
        CaptureConfig {
            mic_device_id: None,
            system_audio: true,
        },
        tx,
        |e| {
            let bar = "#".repeat(((e.rms_db + 60.0).max(0.0) / 2.0) as usize);
            let tag = match e.channel {
                Channel::Me => "Io  ",
                Channel::Them => "Loro",
            };
            println!("{tag} {:7.1} dB {bar}", e.rms_db);
        },
    )
    .expect("avvio cattura");
    std::thread::sleep(Duration::from_secs(secs));
    capture.stop();
    let mut samples = [0usize; 2];
    while let Ok(f) = rx.try_recv() {
        samples[usize::from(f.channel == Channel::Them)] += f.samples.len();
    }
    println!(
        "campioni 16 kHz in coda (ultimi 256 frame): Io={} Loro={}",
        samples[0], samples[1]
    );
}
