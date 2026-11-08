//! Confronto fra modelli locali: per ogni coppia `<nome>.wav` + `<nome>.txt`
//! (testo di riferimento) in una cartella stampa trascrizione, WER, tempo di
//! decodifica e real-time factor. Con `--noise <dB>` aggiunge rumore bianco
//! al rapporto segnale/rumore indicato.
//!
//! cargo run --release -p sv-stt-local --example bench -- <parakeet> <cartella> [--noise 15] [--threads 2] [--provider cpu|coreml]

use std::path::{Path, PathBuf};
use std::time::Instant;

use sv_stt_local::{LocalModel, Recognizer};

fn models_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Application Support/com.giuseppeconcas.sottovoce/models")
}

fn read_wav(path: &Path) -> Vec<f32> {
    let mut r = hound::WavReader::open(path).expect("wav");
    assert_eq!(
        r.spec().sample_rate,
        16_000,
        "{}: serve 16 kHz",
        path.display()
    );
    assert_eq!(r.spec().channels, 1, "{}: serve mono", path.display());
    r.samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / 32768.0)
        .collect()
}

/// Rumore bianco pseudo-casuale (xorshift) al SNR dato.
fn add_noise(samples: &mut [f32], snr_db: f32) {
    let power = samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32;
    let noise_rms = (power / 10f32.powf(snr_db / 10.0)).sqrt();
    let mut x: u32 = 0x9e37_79b9;
    for s in samples {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let u = (x as f32 / u32::MAX as f32) * 2.0 - 1.0;
        *s += u * noise_rms * 3f32.sqrt();
    }
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Word error rate (distanza di edit sulle parole / parole di riferimento).
fn wer(reference: &str, hypothesis: &str) -> f32 {
    let r = words(reference);
    let h = words(hypothesis);
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut cur = vec![i + 1; h.len() + 1];
        for (j, hw) in h.iter().enumerate() {
            let sub = prev[j] + usize::from(rw != hw);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[h.len()] as f32 / r.len().max(1) as f32
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<f32>().ok())
    };
    let (Some(which), Some(dir)) = (args.first(), args.get(1)) else {
        eprintln!("uso: bench <parakeet> <cartella> [--noise dB] [--threads N]");
        std::process::exit(2);
    };
    let threads = flag("--threads").unwrap_or(2.0) as i32;
    let noise = flag("--noise");
    let model = match which.as_str() {
        "parakeet" => LocalModel::Parakeet {
            dir: models_dir().join(sv_models::PARAKEET_V3),
        },
        other => panic!("modello sconosciuto: {other}"),
    };

    let t = Instant::now();
    let provider = args
        .iter()
        .position(|a| a == "--provider")
        .and_then(|i| args.get(i + 1))
        .map_or("cpu", String::as_str);
    let recognizer =
        Recognizer::load_with_provider(&model, threads, provider).expect("caricamento modello");
    println!(
        "{which}: caricato in {} ms ({threads} thread, {provider})",
        t.elapsed().as_millis()
    );

    let mut clips: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("cartella")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    clips.sort();

    let (mut audio_s, mut decode_s, mut wer_sum) = (0.0, 0.0, 0.0);
    // Riscaldamento: la prima inferenza paga l'inizializzazione.
    let _ = recognizer.transcribe(&vec![0.0; 16_000]);
    for wav in &clips {
        let mut samples = read_wav(wav);
        if let Some(db) = noise {
            add_noise(&mut samples, db);
        }
        let reference = std::fs::read_to_string(wav.with_extension("txt")).unwrap_or_default();
        let t = Instant::now();
        let text = recognizer.transcribe(&samples);
        let secs = t.elapsed().as_secs_f32();
        let dur = samples.len() as f32 / 16_000.0;
        let w = wer(&reference, &text);
        audio_s += dur;
        decode_s += secs;
        wer_sum += w;
        println!(
            "{:8} {:5.1}s audio  {:5} ms  RTF {:.3}  WER {:4.1}%  lingua {:?}\n         {text}",
            wav.file_stem().unwrap().to_string_lossy(),
            dur,
            (secs * 1000.0) as u32,
            secs / dur,
            w * 100.0,
            sv_stt_local::lang::guess(&text),
        );
    }
    let n = clips.len().max(1) as f32;
    println!(
        "TOTALE {which}{}: {:.1}s audio, {:.2}s decodifica, RTF {:.3}, WER medio {:.1}%",
        noise
            .map(|d| format!(" (rumore {d} dB)"))
            .unwrap_or_default(),
        audio_s,
        decode_s,
        decode_s / audio_s.max(0.001),
        wer_sum / n * 100.0
    );
}
