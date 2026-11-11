//! Cancellazione d'eco acustica (AEC3 di WebRTC, port Rust `sonora`).
//!
//! Senza cuffie il microfono ("Io") risente dalle casse la voce degli altri.
//! L'audio di sistema catturato ("Loro") è esattamente ciò che le casse
//! suonano: lo usiamo come segnale di riferimento (render) e AEC3 lo toglie
//! dal microfono (capture), stimando da solo ritardo e risposta della stanza.
//!
//! I due canali arrivano da thread diversi con buffering diverso: un blocco
//! del microfono si elabora solo quando il riferimento è arrivato almeno fino
//! alla sua fine (l'eco non precede mai il suono), oppure dopo
//! [`MAX_CAPTURE_WAIT`] di audio in attesa (sistema muto o fermo).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use sonora::config::{EchoCanceller as EchoConfig, TransparentModeType};
use sonora::{AudioProcessing, Config, StreamConfig};

use crate::{AudioFrame, Channel, TARGET_SAMPLE_RATE};

/// Blocco di elaborazione di AEC3: 10 ms.
const BLOCK: usize = TARGET_SAMPLE_RATE as usize / 100;
/// Audio del microfono trattenuto al massimo in attesa del riferimento.
const MAX_CAPTURE_WAIT: Duration = Duration::from_millis(150);
/// Buchi del riferimento oltre cui non si riempie di silenzio (riavvio, ecc.).
const MAX_RENDER_GAP: Duration = Duration::from_secs(1);

/// Cancellatore d'eco di una sessione: riferimento = audio di sistema,
/// uscita = microfono ripulito con gli stessi tempi.
pub struct EchoCanceller {
    apm: AudioProcessing,
    /// Riferimento non ancora multiplo di [`BLOCK`].
    render: Vec<f32>,
    /// Tempo di cattura successivo all'ultimo campione di riferimento ricevuto.
    render_end: Option<Duration>,
    /// Microfono in attesa di elaborazione.
    capture: VecDeque<f32>,
    /// Tempo del primo campione in `capture`.
    capture_start: Option<Duration>,
    out: Vec<f32>,
}

impl Default for EchoCanceller {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoCanceller {
    pub fn new() -> Self {
        Self::with_mode(TransparentModeType::Hmm)
    }

    fn with_mode(transparent_mode: TransparentModeType) -> Self {
        let stream = StreamConfig::new(TARGET_SAMPLE_RATE, 1);
        let apm = AudioProcessing::builder()
            .config(Config {
                echo_canceller: Some(EchoConfig {
                    transparent_mode,
                    ..EchoConfig::default()
                }),
                ..Default::default()
            })
            .capture_config(stream)
            .render_config(stream)
            .build();
        Self {
            apm,
            render: Vec::with_capacity(BLOCK * 4),
            render_end: None,
            capture: VecDeque::with_capacity(BLOCK * 32),
            capture_start: None,
            out: vec![0.0; BLOCK],
        }
    }

    /// Audio di sistema (riferimento dell'eco).
    pub fn push_render(&mut self, frame: &AudioFrame) {
        if frame.samples.is_empty() {
            return;
        }
        // Il tap può tacere quando nessuno suona: il silenzio mancante tiene
        // allineati riferimento e microfono.
        if let Some(end) = self.render_end {
            let gap = frame.start.saturating_sub(end);
            if gap > Duration::from_millis(20) && gap <= MAX_RENDER_GAP {
                let missing = duration_to_samples(gap);
                self.feed_render_samples(&vec![0.0; missing]);
            }
        }
        self.feed_render_samples(&frame.samples);
        self.render_end = Some(frame.start + samples_to_duration(frame.samples.len()));
    }

    /// Audio del microfono; restituisce il microfono ripulito pronto finora
    /// (al più un frame, con il tempo del primo campione).
    pub fn push_capture(&mut self, frame: &AudioFrame) -> Option<AudioFrame> {
        if self.capture.is_empty() {
            self.capture_start = Some(frame.start);
        }
        self.capture.extend(frame.samples.iter());
        self.drain(false)
    }

    /// Fine audio: elabora quanto resta del microfono.
    pub fn flush(&mut self) -> Option<AudioFrame> {
        self.drain(true)
    }

    fn feed_render_samples(&mut self, samples: &[f32]) {
        self.render.extend_from_slice(samples);
        let full = self.render.len() / BLOCK * BLOCK;
        for block in self.render[..full].as_chunks::<BLOCK>().0 {
            // Il render non produce uscita utile: si scrive su `out` e si butta.
            let _ = self.apm.process_render_f32(&[block], &mut [&mut self.out]);
        }
        self.render.drain(..full);
    }

    fn drain(&mut self, all: bool) -> Option<AudioFrame> {
        let start = self.capture_start?;
        let mut cleaned = Vec::new();
        let mut t = start;
        let block_time = samples_to_duration(BLOCK);
        loop {
            let available = self.capture.len();
            if available == 0 || (!all && available < BLOCK) {
                break;
            }
            let pending_end = t + samples_to_duration(available);
            let block_end = t + block_time;
            let reference_ready = self.render_end.is_some_and(|r| r >= block_end);
            if !all && !reference_ready && pending_end.saturating_sub(block_end) < MAX_CAPTURE_WAIT
            {
                break;
            }
            let mut block: Vec<f32> = self.capture.drain(..BLOCK.min(available)).collect();
            let real = block.len();
            block.resize(BLOCK, 0.0);
            match self
                .apm
                .process_capture_f32(&[&block], &mut [&mut self.out])
            {
                Ok(()) => cleaned.extend_from_slice(&self.out[..real]),
                // Mai atteso con formato fisso: meglio l'audio originale che nulla.
                Err(_) => cleaned.extend_from_slice(&block[..real]),
            }
            t = block_end;
        }
        self.capture_start = Some(t);
        (!cleaned.is_empty()).then(|| AudioFrame {
            channel: Channel::Me,
            start,
            samples: Arc::from(cleaned),
        })
    }
}

fn samples_to_duration(n: usize) -> Duration {
    Duration::from_micros(n as u64 * 1_000_000 / u64::from(TARGET_SAMPLE_RATE))
}

fn duration_to_samples(d: Duration) -> usize {
    (d.as_micros() * u128::from(TARGET_SAMPLE_RATE) / 1_000_000) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Segnale simile al parlato: rumore a banda larga (diverso per seme)
    /// con un inviluppo a sillabe e pause.
    fn speechlike(n: usize, seed: u32) -> Vec<f32> {
        let mut state = seed.wrapping_mul(2_654_435_761).max(1);
        let rate = 2.0 + (seed % 5) as f32 * 0.7;
        let mut prev = 0.0f32;
        (0..n)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let noise = (state % 2000) as f32 / 1000.0 - 1.0;
                // Passa-basso leggero: spettro più simile alla voce.
                prev = 0.6 * prev + 0.4 * noise;
                let t = i as f32 / TARGET_SAMPLE_RATE as f32;
                let env = (2.0 * std::f32::consts::PI * rate * t).sin().max(0.0);
                0.3 * env * prev
            })
            .collect()
    }

    fn frame(channel: Channel, at: usize, samples: &[f32]) -> AudioFrame {
        AudioFrame {
            channel,
            start: samples_to_duration(at),
            samples: Arc::from(samples),
        }
    }

    fn energy(s: &[f32]) -> f32 {
        s.iter().map(|x| x * x).sum::<f32>() / s.len().max(1) as f32
    }

    /// Simula 8 s: riferimento a pezzi da 20 ms, microfono = riferimento
    /// ritardato di 40 ms e attenuato (+ eventuale voce propria).
    fn run(own_voice: bool, echo: bool) -> (Vec<f32>, Vec<f32>) {
        run_with(EchoCanceller::new(), 8, own_voice, echo)
    }

    fn run_with(
        mut aec: EchoCanceller,
        secs: usize,
        own_voice: bool,
        echo: bool,
    ) -> (Vec<f32>, Vec<f32>) {
        let n = TARGET_SAMPLE_RATE as usize * secs;
        let far = speechlike(n, 7);
        let near = speechlike(n, 99);
        let delay = TARGET_SAMPLE_RATE as usize * 40 / 1000;
        let mic: Vec<f32> = (0..n)
            .map(|i| {
                let e = if echo && i >= delay {
                    0.5 * far[i - delay]
                } else {
                    0.0
                };
                let v = if own_voice { 0.5 * near[i] } else { 0.0 };
                e + v
            })
            .collect();
        let mut out = Vec::new();
        let chunk = 320;
        for at in (0..n).step_by(chunk) {
            let end = (at + chunk).min(n);
            aec.push_render(&frame(Channel::Them, at, &far[at..end]));
            if let Some(f) = aec.push_capture(&frame(Channel::Me, at, &mic[at..end])) {
                out.extend_from_slice(&f.samples);
            }
        }
        if let Some(f) = aec.flush() {
            out.extend_from_slice(&f.samples);
        }
        (mic, out)
    }

    /// Attenuazione (dB) per finestre da 2 s nei tre scenari.
    #[test]
    #[ignore = "misura, da lanciare a mano con --nocapture"]
    fn measure() {
        let windows = |mic: &[f32], out: &[f32]| -> Vec<String> {
            let w = TARGET_SAMPLE_RATE as usize * 2;
            mic.chunks(w)
                .zip(out.chunks(w))
                .map(|(a, b)| format!("{:.1}", 10.0 * (energy(a) / energy(b).max(1e-12)).log10()))
                .collect()
        };
        for (name, mode) in [
            ("legacy", TransparentModeType::Legacy),
            ("hmm", TransparentModeType::Hmm),
        ] {
            for (case, own, echo) in [
                ("eco", false, true),
                ("voce+riferimento", true, false),
                ("doppio", true, true),
            ] {
                let (mic, out) = run_with(EchoCanceller::with_mode(mode), 20, own, echo);
                println!("{name:6} {case:18} {}", windows(&mic, &out).join(" "));
            }
        }
    }

    #[test]
    fn output_keeps_timing_and_length() {
        let (mic, out) = run(true, false);
        assert_eq!(out.len(), mic.len());
    }

    fn tail_db(a: &[f32], b: &[f32]) -> f32 {
        let tail = a.len() / 2;
        10.0 * (energy(&a[tail..]) / energy(&b[tail..]).max(1e-12)).log10()
    }

    #[test]
    fn echo_is_removed_after_convergence() {
        let (mic, out) = run(false, true);
        let db = tail_db(&mic, &out);
        assert!(db > 30.0, "eco ridotta solo di {db:.1} dB");
    }

    #[test]
    fn own_voice_survives_with_headphones() {
        // Riferimento attivo ma nessuna eco (cuffie): dopo qualche secondo
        // la modalità trasparente lascia passare la voce.
        let (mic, out) = run_with(EchoCanceller::new(), 12, true, false);
        let db = tail_db(&mic, &out);
        assert!(db < 2.0, "voce propria attenuata di {db:.1} dB");
    }

    #[test]
    fn own_voice_survives_double_talk() {
        let n = TARGET_SAMPLE_RATE as usize * 8;
        let voice: Vec<f32> = speechlike(n, 99).iter().map(|x| 0.5 * x).collect();
        let (_, out) = run(true, true);
        let db = tail_db(&voice, &out);
        assert!(db.abs() < 4.0, "voce in doppio parlato: {db:.1} dB");
    }

    #[test]
    fn capture_waits_for_reference_then_gives_up() {
        let mut aec = EchoCanceller::new();
        let mic = speechlike(BLOCK * 4, 3);
        // Nessun riferimento: trattiene fino a MAX_CAPTURE_WAIT di audio.
        assert!(aec.push_capture(&frame(Channel::Me, 0, &mic)).is_none());
        let more = speechlike(BLOCK * 20, 4);
        let got = aec
            .push_capture(&frame(Channel::Me, BLOCK * 4, &more))
            .unwrap();
        assert_eq!(got.start, Duration::ZERO);
        assert!(!got.samples.is_empty());
    }
}
