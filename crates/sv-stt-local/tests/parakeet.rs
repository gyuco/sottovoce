//! Motore locale reale (Silero VAD + Parakeet v3) su due canali in parallelo.
//! Servono i modelli nella cartella dell'app
//! (`cargo run -p sv-models --example fetch -- silero-vad` e
//! `... -- parakeet-tdt-0.6b-v3-int8`): senza, il test viene saltato.
//! In debug è lento: `cargo test --release -p sv-stt-local`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sv_audio::{AudioFrame, Channel};
use sv_stt::{Language, Segment, SttEngine, SttEventKind};
use sv_stt_local::{LocalEngine, LocalEngineConfig, LocalModel};
use sv_vad::VadConfig;

fn models() -> Option<(PathBuf, PathBuf)> {
    let dir = PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Application Support/com.giuseppeconcas.sottovoce/models");
    let vad = dir.join(sv_models::SILERO_VAD).join("silero_vad.onnx");
    let parakeet = dir.join(sv_models::PARAKEET_V3);
    if vad.is_file() && parakeet.join("encoder.int8.onnx").is_file() {
        Some((vad, parakeet))
    } else {
        eprintln!(
            "SALTATO: modelli Silero VAD / Parakeet assenti in {}",
            dir.display()
        );
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

async fn transcribe(engine: &LocalEngine, channel: Channel, audio: Arc<Vec<f32>>) -> Vec<Segment> {
    let mut stream = engine.start(channel, Language::Multi);
    let sender = stream.audio.clone();
    drop(stream.audio);
    let feed = tokio::spawn(async move {
        for (i, chunk) in audio.chunks(1024).enumerate() {
            let frame = AudioFrame {
                channel,
                start: Duration::from_micros((i * 1024) as u64 * 1_000_000 / 16_000),
                samples: Arc::from(chunk),
            };
            if sender.send(frame).await.is_err() {
                break;
            }
            // Tempo reale accelerato 8×: le frasi arrivano mentre si decodifica.
            tokio::time::sleep(Duration::from_millis(8)).await;
        }
    });
    let mut finals = Vec::new();
    while let Some(e) = stream.events.recv().await {
        match e.kind {
            SttEventKind::Final(s) => finals.push(s),
            SttEventKind::Error { message } => panic!("{channel:?}: {message}"),
            _ => {}
        }
    }
    feed.await.unwrap();
    finals
}

#[tokio::test(flavor = "multi_thread")]
async fn parakeet_transcribes_two_channels_in_parallel() {
    let Some((vad, parakeet)) = models() else {
        return;
    };
    let engine = LocalEngine::new(LocalEngineConfig {
        model: LocalModel::Parakeet { dir: parakeet },
        vad: VadConfig::new(vad),
        num_threads: 2,
    });
    engine.ready().await.unwrap();

    let audio = Arc::new(fixture());
    let (me, them) = tokio::join!(
        transcribe(&engine, Channel::Me, audio.clone()),
        transcribe(&engine, Channel::Them, audio)
    );

    for finals in [&me, &them] {
        assert_eq!(finals.len(), 2, "{finals:?}");
        let text = |s: &Segment| s.text.to_lowercase();
        assert!(
            text(&finals[0]).contains("riunione"),
            "{:?}",
            finals[0].text
        );
        assert!(
            text(&finals[1]).contains("progetto"),
            "{:?}",
            finals[1].text
        );
        assert!(finals.iter().all(|s| s.language.as_deref() == Some("it")));
    }
}
