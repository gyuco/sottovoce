//! Microfono via `cpal`.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat};
use serde::Serialize;

use crate::error::AudioError;
use crate::source::{OpenedSource, push_converted, ring};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// Elenca i dispositivi di input disponibili.
pub fn list_input_devices() -> Result<Vec<InputDevice>, AudioError> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|d| d.id().ok())
        .map(|id| id.to_string());
    let mut out = Vec::new();
    for dev in host.input_devices()? {
        let Ok(id) = dev.id().map(|id| id.to_string()) else {
            continue;
        };
        let name = dev
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| id.clone());
        out.push(InputDevice {
            is_default: default_id.as_deref() == Some(id.as_str()),
            id,
            name,
        });
    }
    Ok(out)
}

fn find_device(device_id: Option<&str>) -> Result<Device, AudioError> {
    let host = cpal::default_host();
    match device_id {
        None => host.default_input_device().ok_or(AudioError::NoInputDevice),
        Some(id) => {
            let parsed = id
                .parse()
                .map_err(|_| AudioError::DeviceNotFound(id.to_owned()))?;
            host.device_by_id(&parsed)
                .ok_or_else(|| AudioError::DeviceNotFound(id.to_owned()))
        }
    }
}

/// Apre il microfono e avvia lo stream. Va chiamata nel thread che possiede lo stream.
pub(crate) fn open(device_id: Option<&str>) -> Result<OpenedSource, AudioError> {
    let device = find_device(device_id)?;
    let supported = device.default_input_config()?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let (mut prod, consumer) = ring(config.sample_rate, usize::from(config.channels));

    let on_err = |e: cpal::Error| tracing::warn!("stream microfono: {e}");
    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
            config,
            move |data, _| push_converted(&mut prod, data, |s| s),
            on_err,
            None,
        )?,
        SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
            config,
            move |data, _| push_converted(&mut prod, data, |s| f32::from(s) / 32_768.0),
            on_err,
            None,
        )?,
        SampleFormat::I32 => device.build_input_stream::<i32, _, _>(
            config,
            move |data, _| push_converted(&mut prod, data, |s| s as f32 / 2_147_483_648.0),
            on_err,
            None,
        )?,
        other => return Err(AudioError::UnsupportedFormat(other.to_string())),
    };
    stream.play()?;

    Ok(OpenedSource {
        sample_rate: config.sample_rate,
        channels: usize::from(config.channels),
        consumer,
        _guard: Box::new(stream),
    })
}
