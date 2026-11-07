use std::any::Any;

use ringbuf::traits::{Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

/// Sorgente aperta e in esecuzione. Finché `_guard` vive, il callback
/// realtime scrive campioni interleaved f32 nel ring buffer.
pub struct OpenedSource {
    pub sample_rate: u32,
    pub channels: usize,
    pub consumer: HeapCons<f32>,
    /// Risorse della piattaforma (stream cpal, tap Core Audio...). Il drop ferma la cattura.
    pub _guard: Box<dyn Any>,
}

/// Ring buffer dimensionato per ~2 secondi di audio.
pub fn ring(sample_rate: u32, channels: usize) -> (HeapProd<f32>, HeapCons<f32>) {
    HeapRb::<f32>::new(sample_rate as usize * channels * 2).split()
}

/// Converte campioni in f32 e li spinge nel ring buffer senza allocare
/// (sicuro per il thread realtime). I campioni in eccesso vengono scartati.
pub fn push_converted<T: Copy>(prod: &mut HeapProd<f32>, data: &[T], convert: impl Fn(T) -> f32) {
    let mut buf = [0.0f32; 512];
    for chunk in data.chunks(buf.len()) {
        for (dst, &src) in buf.iter_mut().zip(chunk) {
            *dst = convert(src);
        }
        if prod.push_slice(&buf[..chunk.len()]) < chunk.len() {
            // Buffer pieno: il worker è in ritardo. Meglio perdere audio che bloccare.
            break;
        }
    }
}
