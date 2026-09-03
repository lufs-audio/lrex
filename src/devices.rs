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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Numerical quality rank for CPAL sample formats. Higher rank means higher
/// resolution or headroom.
pub fn sample_format_quality_rank(f: SampleFormat) -> u8 {
    match f {
        SampleFormat::F32 | SampleFormat::F64 => 5,
        SampleFormat::I32 | SampleFormat::U32 | SampleFormat::I64 | SampleFormat::U64 => 4,
        SampleFormat::I16 | SampleFormat::U16 => 3,
        SampleFormat::I8 => 1,
        SampleFormat::U8 => 0,
        _ => 0,
    }
}

/// Whether a sample format is high-resolution (>= 16-bit).
/// Low-resolution formats (< 16-bit, like 8-bit unsigned) introduce severe
/// quantization noise and must never be preferred over high-resolution formats.
pub fn sample_format_is_high_res(f: SampleFormat) -> bool {
    sample_format_quality_rank(f) >= 3
}

/// Decide if candidate config `cand` is strictly better than current `best`.
pub fn is_better_config(
    cand: &ChosenConfig,
    best: &ChosenConfig,
    def_format: Option<SampleFormat>,
) -> bool {
    let cand_hi = sample_format_is_high_res(cand.sample_format);
    let best_hi = sample_format_is_high_res(best.sample_format);

    // 1. High-resolution (>= 16-bit) formats always beat low-resolution (8-bit)
    // formats, regardless of channel count (never sacrifice bit depth just to
    // save a channel).
    if cand_hi != best_hi {
        return cand_hi;
    }

    // 2. If both are high-res (or both low-res), prefer fewer channels that
    // still satisfy the request (don't open a 32-ch interface if 2 will do).
    if cand.channels != best.channels {
        return cand.channels < best.channels;
    }

    // 3. For equal channel counts, prefer matching the device's default format.
    let cand_matches_def = def_format
        .map(|df| df == cand.sample_format)
        .unwrap_or(false);
    let best_matches_def = def_format
        .map(|df| df == best.sample_format)
        .unwrap_or(false);
    if cand_matches_def != best_matches_def {
        return cand_matches_def;
    }

    // 4. For equal channel counts, prefer higher quality format (f32 > i32 > i16).
    let cand_rank = sample_format_quality_rank(cand.sample_format);
    let best_rank = sample_format_quality_rank(best.sample_format);
    cand_rank > best_rank
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
    let def_config = device.default_input_config().ok();

    // Prefer the device default if it already satisfies the request.
    if let Some(def) = &def_config {
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

    let def_format = def_config.as_ref().map(|d| d.sample_format());
    let def_sr = def_config.as_ref().map(|d| d.sample_rate().0);

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
            None => match def_sr {
                Some(dsr) if dsr >= r.min_sample_rate().0 && dsr <= r.max_sample_rate().0 => dsr,
                _ => r.max_sample_rate().0,
            },
        };
        let cand = ChosenConfig {
            channels: r.channels(),
            sample_rate: sr,
            sample_format: r.sample_format(),
        };

        best = Some(match best {
            Some(b) if !is_better_config(&cand, &b, def_format) => b,
            _ => cand,
        });
    }

    best.ok_or_else(|| {
        let at = want_rate.map(|r| format!(" at {r} Hz")).unwrap_or_default();
        format!("no input config provides >= {max_channel_needed} channels{at}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_res_always_beats_low_res_regardless_of_channels() {
        let u8_mono = ChosenConfig {
            channels: 1,
            sample_rate: 48000,
            sample_format: SampleFormat::U8,
        };
        let f32_stereo = ChosenConfig {
            channels: 2,
            sample_rate: 48000,
            sample_format: SampleFormat::F32,
        };
        let i16_mono = ChosenConfig {
            channels: 1,
            sample_rate: 48000,
            sample_format: SampleFormat::I16,
        };

        // High-res (2ch F32) beats low-res (1ch U8) even though U8 has fewer channels
        assert!(is_better_config(&f32_stereo, &u8_mono, None));
        assert!(!is_better_config(&u8_mono, &f32_stereo, None));

        // High-res (1ch I16) beats low-res (1ch U8)
        assert!(is_better_config(&i16_mono, &u8_mono, None));
        assert!(!is_better_config(&u8_mono, &i16_mono, None));
    }

    #[test]
    fn prefers_fewer_channels_when_both_high_res() {
        let f32_2ch = ChosenConfig {
            channels: 2,
            sample_rate: 48000,
            sample_format: SampleFormat::F32,
        };
        let f32_8ch = ChosenConfig {
            channels: 8,
            sample_rate: 48000,
            sample_format: SampleFormat::F32,
        };
        let i16_2ch = ChosenConfig {
            channels: 2,
            sample_rate: 48000,
            sample_format: SampleFormat::I16,
        };
        let i32_16ch = ChosenConfig {
            channels: 16,
            sample_rate: 48000,
            sample_format: SampleFormat::I32,
        };

        assert!(is_better_config(&f32_2ch, &f32_8ch, None));
        assert!(!is_better_config(&f32_8ch, &f32_2ch, None));

        // 2ch I16 beats 16ch I32 because both are high-res and 2ch is far more channel-efficient
        assert!(is_better_config(&i16_2ch, &i32_16ch, None));
    }

    #[test]
    fn prefers_device_default_format_on_channel_tie() {
        let f32_stereo = ChosenConfig {
            channels: 2,
            sample_rate: 48000,
            sample_format: SampleFormat::F32,
        };
        let i32_stereo = ChosenConfig {
            channels: 2,
            sample_rate: 48000,
            sample_format: SampleFormat::I32,
        };

        // When default format is I32, I32 beats F32 on channel tie
        assert!(is_better_config(
            &i32_stereo,
            &f32_stereo,
            Some(SampleFormat::I32)
        ));
        assert!(!is_better_config(
            &f32_stereo,
            &i32_stereo,
            Some(SampleFormat::I32)
        ));

        // When default format is F32, F32 beats I32
        assert!(is_better_config(
            &f32_stereo,
            &i32_stereo,
            Some(SampleFormat::F32)
        ));
        assert!(!is_better_config(
            &i32_stereo,
            &f32_stereo,
            Some(SampleFormat::F32)
        ));
    }

    #[test]
    fn prefers_higher_quality_format_as_tiebreaker() {
        let f32_mono = ChosenConfig {
            channels: 1,
            sample_rate: 48000,
            sample_format: SampleFormat::F32,
        };
        let i16_mono = ChosenConfig {
            channels: 1,
            sample_rate: 48000,
            sample_format: SampleFormat::I16,
        };
        let i32_mono = ChosenConfig {
            channels: 1,
            sample_rate: 48000,
            sample_format: SampleFormat::I32,
        };

        // Without a default format bias, F32 > I32 > I16 on equal channels
        assert!(is_better_config(&f32_mono, &i16_mono, None));
        assert!(is_better_config(&f32_mono, &i32_mono, None));
        assert!(is_better_config(&i32_mono, &i16_mono, None));
    }

    #[test]
    fn choose_input_config_selects_high_res_at_48khz_on_default_device() {
        let host = cpal::default_host();
        if let Some(dev) = host.default_input_device() {
            if let Ok(chosen) = choose_input_config(&dev, Some(48000), 1) {
                assert!(
                    sample_format_is_high_res(chosen.sample_format),
                    "selected format {:?} is not high-res!",
                    chosen.sample_format
                );
                assert_eq!(chosen.sample_rate, 48000);
                assert!(chosen.channels >= 1);
            }
        }
    }
}
