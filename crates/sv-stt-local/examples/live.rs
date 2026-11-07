//! Prova da terminale: trascrizione live con il motore locale.
//!
//! cargo run --release -p sv-stt-local --example live -- [secondi] [--system]
//!
//! Senza `--system` trascrive solo il microfono ("Io"). Modelli dalla
//! cartella dell'app (`cargo run -p sv-models --example fetch -- <id>`).

use std::time::Duration;

use sv_audio::{AudioFrame, Capture, CaptureConfig, Channel};
use sv_stt::{Language, SttEngine, SttEventKind};
use sv_stt_local::{LocalEngine, LocalEngineConfig, LocalModel};
use sv_vad::VadConfig;
use tokio::sync::broadcast;
use tokio::sync::mpsc::error::TrySendError;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("info,sv_stt_local=debug")
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let secs: u64 = args.iter().find_map(|a| a.parse().ok()).unwrap_or(30);
    let system_audio = args.iter().any(|a| a == "--system");
    let models = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Application Support/com.giuseppeconcas.sottovoce/models");
    let model = LocalModel::Parakeet {
        dir: models.join(sv_models::PARAKEET_V3),
    };
    let engine = LocalEngine::new(LocalEngineConfig {
        model,
        vad: VadConfig::new(models.join(sv_models::SILERO_VAD).join("silero_vad.onnx")),
        num_threads: 2,
    });
    if let Err(e) = engine.ready().await {
        eprintln!("{e}");
        std::process::exit(1);
    }

    let (tx, _) = broadcast::channel::<AudioFrame>(512);
    let mut channels = vec![Channel::Me];
    if system_audio {
        channels.push(Channel::Them);
    }

    let mut tasks = Vec::new();
    for ch in channels {
        let lang = Language::Multi;
        let mut stream = engine.start(ch, lang);
        let mut frames = tx.subscribe();
        let audio = stream.audio.clone();
        // broadcast → stream (solo il canale giusto). Chiude quando il bus chiude.
        tasks.push(tokio::spawn(async move {
            loop {
                match frames.recv().await {
                    Ok(f) if f.channel == ch => match audio.try_send(f) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => eprintln!("{ch:?}: coda audio piena"),
                        // Motore terminato (es. errore di autenticazione).
                        Err(TrySendError::Closed(_)) => break,
                    },
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("{ch:?}: persi {n} frame");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }));
        drop(stream.audio);
        tasks.push(tokio::spawn(async move {
            while let Some(e) = stream.events.recv().await {
                let tag = if e.channel == Channel::Me {
                    "Io  "
                } else {
                    "Loro"
                };
                match e.kind {
                    SttEventKind::Partial(s) => println!("{tag} … {}", s.text),
                    SttEventKind::Final(s) => println!(
                        "{tag} ✔ [{:.1}s–{:.1}s] {}",
                        s.start.as_secs_f32(),
                        s.end.as_secs_f32(),
                        s.text
                    ),
                    other => println!("{tag} · {other:?}"),
                }
            }
        }));
    }

    let capture = Capture::start(
        CaptureConfig {
            mic_device_id: None,
            system_audio,
        },
        tx,
        |_| {},
    )
    .expect("avvio cattura");
    tokio::time::sleep(Duration::from_secs(secs)).await;
    // Fermare la cattura chiude il bus → audio chiuso → ultime frasi → Disconnected.
    capture.stop();
    for t in tasks {
        let _ = t.await;
    }
}
