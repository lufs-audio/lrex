//! Audio device enumeration and config resolution (cpal).
//!
//! `devices` emits a machine-readable inventory; `record` resolves a requested
//! device + format against what the hardware actually supports, failing with the
//! right exit code when it can't (device unavailable vs. format unsupported).

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use cpal::SampleFormat;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_default: bool,
    pub max_input_channels: u16,
    pub default_sample_rate: u32,
    pub default_sample_format: String,
    pub sample_rate_range: [u32; 2],
    pub sample_formats: Vec<String>,
}

/// The format/channel choice made for a capture stream.
#[derive(Debug, Clone, Copy)]
pub struct ChosenConfig {
    pub channels: u16,
    pub sample_rate: u32,
    pub sample_format: SampleFormat,
}

pub fn sample_format_name(f: SampleFormat) -> String {
    match f {
        SampleFormat::I8 => "i8",
        SampleFormat::I16 => "i16",
        SampleFormat::I32 => "i32",
        SampleFormat::I64 => "i64",
        SampleFormat::U8 => "u8",
        SampleFormat::U16 => "u16",
        SampleFormat::U32 => "u32",
        SampleFormat::U64 => "u64",
        SampleFormat::F32 => "f32",
        SampleFormat::F64 => "f64",
        _ => "other",
    }
    .to_string()
}

/// Enumerate input devices and their capabilities.
pub fn list_input_devices() -> Result<Vec<AudioDeviceInfo>> {
    let host = cpal::default_host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());

    let mut out = Vec::new();
    for dev in host.input_devices().context("enumerating input devices")? {
        let name = dev.name().unwrap_or_else(|_| "<unknown>".to_string());

        let mut max_ch: u16 = 0;
        let mut min_sr: u32 = u32::MAX;
        let mut max_sr: u32 = 0;
        let mut formats: BTreeSet<String> = BTreeSet::new();

        if let Ok(cfgs) = dev.supported_input_configs() {
            for c in cfgs {
                max_ch = max_ch.max(c.channels());
                min_sr = min_sr.min(c.min_sample_rate().0);
                max_sr = max_sr.max(c.max_sample_rate().0);
                formats.insert(sample_format_name(c.sample_format()));
            }
        }
        if min_sr == u32::MAX {
            min_sr = 0;
        }

        let (def_sr, def_fmt) = match dev.default_input_config() {
            Ok(d) => (d.sample_rate().0, sample_format_name(d.sample_format())),
            Err(_) => (0, "unknown".to_string()),
        };

        out.push(AudioDeviceInfo {
            is_default: default_name.as_deref() == Some(name.as_str()),
            name,
            max_input_channels: max_ch,
            default_sample_rate: def_sr,
            default_sample_format: def_fmt,
            sample_rate_range: [min_sr, max_sr],
            sample_formats: formats.into_iter().collect(),
        });
    }
    Ok(out)
}

/// Resolve a device by "default", exact name, or case-insensitive substring.
/// Returns a human message on failure (mapped to exit 3 by the caller).
pub fn resolve_input_device(query: &str) -> std::result::Result<cpal::Device, String> {
    let host = cpal::default_host();
    if query == "default" {
        return host
            .default_input_device()
            .ok_or_else(|| "no default audio input device".to_string());
    }
    let devices: Vec<cpal::Device> = host
        .input_devices()
        .map_err(|e| format!("enumerating input devices: {e}"))?
        .collect();

    if let Some(d) = devices
        .iter()
        .find(|d| d.name().map(|n| n == query).unwrap_or(false))
    {
        return Ok(d.clone());
    }
    let needle = query.to_lowercase();
    if let Some(d) = devices.iter().find(|d| {
        d.name()
            .map(|n| n.to_lowercase().contains(&needle))
            .unwrap_or(false)
    }) {
        return Ok(d.clone());
    }
    Err(format!("audio input device '{query}' not found"))
}

/// Choose a supported input config that provides at least `max_channel_needed`
/// channels (so a channel like 9 or 10 exists to de-interleave) at the desired
/// rate. Returns a human message on failure (mapped to exit 4 by the caller).
pub fn choose_input_config(
    device: &cpal::Device,
    want_rate: Option<u32>,
    max_channel_needed: u16,
) -> std::result::Result<ChosenConfig, String> {
    // Prefer the device default if it already satisfies the request.
    if let Ok(def) = device.default_input_config() {
        let sr = def.sample_rate().0;
        let rate_ok = want_rate.map(|r| r == sr).unwrap_or(true);
        if def.channels() >= max_channel_needed && rate_ok {
            return Ok(ChosenConfig {
                channels: def.channels(),
                sample_rate: sr,
                sample_format: def.sample_format(),
            });
        }
    }

    let ranges: Vec<cpal::SupportedStreamConfigRange> = device
        .supported_input_configs()
        .map_err(|e| format!("querying supported input configs: {e}"))?
        .collect();
    if ranges.is_empty() {
        return Err("device reports no supported input configs".to_string());
    }

    let mut best: Option<ChosenConfig> = None;
    for r in &ranges {
        if r.channels() < max_channel_needed {
            continue;
        }
        let sr = match want_rate {
            Some(rr) => {
                if rr >= r.min_sample_rate().0 && rr <= r.max_sample_rate().0 {
                    rr
                } else {
                    continue;
                }
            }
            None => r.max_sample_rate().0,
        };
        let cand = ChosenConfig {
            channels: r.channels(),
            sample_rate: sr,
            sample_format: r.sample_format(),
        };
        // Prefer the fewest channels that still cover the request (don't open a
        // 64-in interface wide-open if 10 will do).
        best = Some(match best {
            Some(b) if b.channels <= cand.channels => b,
            _ => cand,
        });
    }

    best.ok_or_else(|| {
        let at = want_rate.map(|r| format!(" at {r} Hz")).unwrap_or_default();
        format!("no input config provides >= {max_channel_needed} channels{at}")
    })
}
