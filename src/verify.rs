//! The verification contract — turns "a take ran" into "a take is proven".
//!
//! Re-derives the verification result from the files on disk versus the claims
//! in the manifest. Used both inline after `record` and standalone via
//! `verify <take-dir>`. `verified` is true only when every *gating* check passes.
//!
//! Multi-device (v0.5): every check below still runs at the whole-take level
//! (preserving the exact check names/semantics a single-device take always
//! had), AND now also breaks xrun/rate/bit-depth/decode/exists down per device
//! (`device_*` checks) so a multi-device take's failure names WHICH device
//! glitched instead of only failing the take as an undifferentiated whole. A
//! single-device take is just the one-device case of the same per-device
//! logic — there is no separate code path for it.

use crate::manifest::{Check, Manifest, Verification};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

/// Duration tolerance: the larger of 0.5s or 5% of the expected length.
fn duration_tolerance(expected_s: f64) -> f64 {
    (expected_s * 0.05).max(0.5)
}

/// Sane upper bound for the applied MIDI→audio alignment shift. The shift is
/// `audio_t0 − input_latency` — normally ~50–150 ms (device warm-up + buffer).
/// Well outside this range means the anchor is broken or audio never started.
const AV_SHIFT_MAX_MS: f64 = 500.0;

struct WavStats {
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
    is_float: bool,
    frames: u64,
    peak_dbfs: f32,
}

fn dbfs(x: f32) -> f32 {
    if x <= 0.0 {
        -f32::INFINITY
    } else {
        20.0 * x.log10()
    }
}

fn read_wav_stats(path: &Path) -> Result<WavStats> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = reader.spec();
    let channels = spec.channels;

    let mut peak: f32 = 0.0;
    let mut n: u64 = 0;

    match spec.sample_format {
        hound::SampleFormat::Float => {
            for s in reader.samples::<f32>() {
                let v = s.context("decoding float sample")?;
                peak = peak.max(v.abs());
                n += 1;
            }
        }
        hound::SampleFormat::Int => {
            let denom = 2f64.powi(spec.bits_per_sample as i32 - 1);
            for s in reader.samples::<i32>() {
                let raw = s.context("decoding int sample")?;
                let v = (raw as f64 / denom) as f32;
                peak = peak.max(v.abs());
                n += 1;
            }
        }
    }

    let frames = if channels == 0 {
        0
    } else {
        n / channels as u64
    };

    Ok(WavStats {
        channels,
        sample_rate: spec.sample_rate,
        bits_per_sample: spec.bits_per_sample,
        is_float: matches!(spec.sample_format, hound::SampleFormat::Float),
        frames,
        peak_dbfs: dbfs(peak),
    })
}

fn expected_bits(bit_depth: &str) -> (u16, bool) {
    match bit_depth {
        "16" => (16, false),
        "24" => (24, false),
        "32f" => (32, true),
        _ => (0, false),
    }
}

/// Per-device rollup of the file/decode/format dimensions, used to emit
/// `device_*` checks alongside the whole-take aggregate ones.
///
/// Deliberately NOT `#[derive(Default)]`: this type's correct initial state is
/// "innocent until proven guilty" (every dimension starts `true` and only
/// flips to `false` on an observed failure), which is the opposite of the
/// derived `Default` (all-`false`). A stray `or_default()` here would silently
/// report every device as already-failed — see `PerDevice::new`.
struct PerDevice {
    exists: bool,
    decodes: bool,
    rate_ok: bool,
    depth_ok: bool,
}

impl PerDevice {
    fn new() -> Self {
        PerDevice {
            exists: true,
            decodes: true,
            rate_ok: true,
            depth_ok: true,
        }
    }
}

/// Run the contract against the files in `take_dir`, comparing to `manifest`.
pub fn run(take_dir: &Path, manifest: &Manifest) -> Verification {
    let mut checks: Vec<Check> = Vec::new();

    let (want_bits, want_float) = expected_bits(&manifest.requested.bit_depth);

    // --- Per-track file + decode + format checks ---
    let mut all_exist = true;
    let mut all_decode = true;
    let mut channels_match = manifest.tracks.len() == manifest.requested.tracks.len();
    let mut rate_match = true;
    let mut depth_match = true;
    let mut total_frames: Option<u64> = None;
    let mut min_peak_dbfs = f32::INFINITY;
    let mut max_peak_dbfs = f32::NEG_INFINITY;

    // Keyed by device name (BTreeMap so check order is deterministic across
    // runs, which matters for stable, diffable verification reports).
    let mut per_device: BTreeMap<String, PerDevice> = BTreeMap::new();

    for (track, req) in manifest.tracks.iter().zip(manifest.requested.tracks.iter()) {
        let dev = per_device
            .entry(track.device.clone())
            .or_insert_with(PerDevice::new);

        let path = take_dir.join(&track.file);
        let nonempty = std::fs::metadata(&path)
            .map(|m| m.len() > 44)
            .unwrap_or(false);
        if !nonempty {
            all_exist = false;
            dev.exists = false;
            continue;
        }
        match read_wav_stats(&path) {
            Ok(st) => {
                if st.channels as usize != req.channels.len() {
                    channels_match = false;
                }
                if st.sample_rate != manifest.captured.rate {
                    rate_match = false;
                    dev.rate_ok = false;
                }
                if st.bits_per_sample != want_bits || st.is_float != want_float {
                    depth_match = false;
                    dev.depth_ok = false;
                }
                match total_frames {
                    None => total_frames = Some(st.frames),
                    Some(f) if f != st.frames => total_frames = Some(f.min(st.frames)),
                    _ => {}
                }
                min_peak_dbfs = min_peak_dbfs.min(st.peak_dbfs);
                max_peak_dbfs = max_peak_dbfs.max(st.peak_dbfs);
            }
            Err(_) => {
                all_decode = false;
                dev.decodes = false;
            }
        }
    }

    checks.push(Check::gating(
        "files_exist_nonempty",
        all_exist,
        (!all_exist).then(|| "one or more track files missing or empty".to_string()),
    ));
    checks.push(Check::gating(
        "audio_decodes",
        all_decode,
        (!all_decode).then(|| "one or more WAVs failed to decode".to_string()),
    ));
    checks.push(Check::gating(
        "channel_count_matches",
        channels_match,
        (!channels_match).then(|| "captured channel layout != requested".to_string()),
    ));
    checks.push(Check::gating(
        "sample_rate_matches",
        rate_match,
        (!rate_match).then(|| format!("a WAV rate != captured {} Hz", manifest.captured.rate)),
    ));
    checks.push(Check::gating(
        "bit_depth_matches",
        depth_match,
        (!depth_match)
            .then(|| format!("a WAV depth != requested {}", manifest.requested.bit_depth)),
    ));

    // --- Per-device breakdown of the four checks above. For a single-device
    // take this is exactly one check per dimension, true under the identical
    // condition as the aggregate above — no new failure mode, just attribution.
    for (device, pd) in &per_device {
        checks.push(Check::gating(
            "device_files_exist_nonempty",
            pd.exists,
            (!pd.exists).then(|| format!("device '{device}': a track file is missing or empty")),
        ));
        checks.push(Check::gating(
            "device_audio_decodes",
            pd.decodes,
            (!pd.decodes).then(|| format!("device '{device}': a track WAV failed to decode")),
        ));
        checks.push(Check::gating(
            "device_rate_matches",
            pd.rate_ok,
            (!pd.rate_ok).then(|| {
                format!(
                    "device '{device}': a track's WAV rate != captured {} Hz",
                    manifest.captured.rate
                )
            }),
        ));
        checks.push(Check::gating(
            "device_bit_depth_matches",
            pd.depth_ok,
            (!pd.depth_ok).then(|| {
                format!(
                    "device '{device}': a track's WAV depth != requested {}",
                    manifest.requested.bit_depth
                )
            }),
        ));
    }

    // --- Duration sanity ---
    let frames = total_frames.unwrap_or(0);
    let dur_s = if manifest.captured.rate > 0 {
        frames as f64 / manifest.captured.rate as f64
    } else {
        0.0
    };
    let duration_ok = if let Some(req_dur) = manifest.requested.duration_s {
        frames > 0 && (dur_s - req_dur).abs() <= duration_tolerance(req_dur)
    } else {
        frames > 0
    };
    checks.push(Check::gating(
        "duration_sane",
        duration_ok,
        (!duration_ok).then(|| format!("captured {dur_s:.3}s ({frames} frames)")),
    ));

    // --- The heart of the contract: no dropped frames (aggregate, all devices) ---
    let no_xruns = manifest.captured.xruns == 0;
    checks.push(Check::gating(
        "no_xruns",
        no_xruns,
        (!no_xruns).then(|| {
            format!(
                "{} dropped-frame/overflow event(s)",
                manifest.captured.xruns
            )
        }),
    ));
    // Per-device breakdown — names which device glitched. Always one entry per
    // captured device, even for a single-device take (mirrors `no_xruns`).
    for dx in &manifest.captured.xruns_by_device {
        checks.push(Check::gating(
            "device_no_xruns",
            dx.xruns == 0,
            (dx.xruns != 0).then(|| {
                format!(
                    "device '{}': {} dropped-frame/overflow event(s)",
                    dx.device, dx.xruns
                )
            }),
        ));
    }

    // --- Loudness sanity (not digital silence) ---
    let not_silent = min_peak_dbfs > -90.0;
    checks.push(Check::gating(
        "loudness_not_silent",
        not_silent,
        (!not_silent).then(|| {
            format!(
                "quietest track peak {min_peak_dbfs:.1} dBFS (looks like silence — wrong channel?)"
            )
        }),
    ));
    // Clipping is reported, not gated (a hot take isn't necessarily wrong).
    let not_clipping = max_peak_dbfs < -0.1;
    checks.push(Check::info(
        "not_clipping",
        not_clipping,
        Some(format!("loudest track peak {max_peak_dbfs:.1} dBFS")),
    ));

    // --- MIDI integrity (only if armed and events occurred) ---
    if let Some(midi) = &manifest.midi {
        let mid_path = take_dir.join(&midi.file);
        let mid_exists = std::fs::metadata(&mid_path)
            .map(|m| m.len() > 0)
            .unwrap_or(false);
        checks.push(Check::gating(
            "midi_file_exists",
            mid_exists,
            (!mid_exists).then(|| format!("{} missing or empty", midi.file)),
        ));
        let balanced = midi.note_ons == midi.note_offs;
        checks.push(Check::gating(
            "midi_notes_balanced",
            balanced,
            (!balanced)
                .then(|| format!("note-ons {} != note-offs {}", midi.note_ons, midi.note_offs)),
        ));
    }

    // --- A/V alignment. The precise anchor math is gated tightly by `selftest`
    // (in-process, sub-ms). Here, on a live take with MIDI armed, we gate that
    // the applied MIDI→audio shift is *sane* (a broken/absent anchor would sit
    // outside this bound). The sub-frame residual — MIDI transport jitter + the
    // instrument's own note latency — is not something we can measure live, so
    // it is not gated. MIDI is anchored against the take's primary device only
    // (see record::run) — this check is therefore whole-take, not per-device.
    if manifest.midi.is_some() {
        let shift = manifest.captured.av_offset_ms;
        let ok = (0.0..=AV_SHIFT_MAX_MS).contains(&shift);
        checks.push(Check::gating(
            "av_offset_within_tol",
            ok,
            (!ok).then(|| {
                format!("MIDI→audio shift {shift:.2} ms outside [0, {AV_SHIFT_MAX_MS:.0}] ms")
            }),
        ));
    } else {
        checks.push(Check::info(
            "av_offset_within_tol",
            true,
            Some("no MIDI armed; A/V alignment not applicable".to_string()),
        ));
    }

    let verified = checks.iter().all(|c| !c.gating || c.ok);
    Verification { verified, checks }
}

/// Load a take's manifest and re-run the contract against its files.
pub fn verify_dir(take_dir: &Path) -> Result<(Manifest, Verification)> {
    let manifest_path = take_dir.join("take.json");
    let text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let mut manifest: Manifest = serde_json::from_str(&text)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;
    let verification = run(take_dir, &manifest);
    manifest.verification = verification.clone();
    Ok((manifest, verification))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Captured, DeviceXruns, MidiInfo, Requested, RequestedTrack, TrackInfo};

    /// Build a minimal, otherwise-valid manifest for a take with the given
    /// per-device xrun counts, without touching disk — these tests exercise
    /// the xrun/attribution logic directly, independent of `read_wav_stats`.
    fn manifest_with_xruns(xruns_by_device: Vec<DeviceXruns>) -> Manifest {
        let total: u64 = xruns_by_device.iter().map(|d| d.xruns).sum();
        let devices: Vec<String> = xruns_by_device.iter().map(|d| d.device.clone()).collect();
        Manifest {
            schema_version: crate::manifest::SCHEMA_VERSION,
            tool_version: "test".to_string(),
            take_id: "rec-00000000".to_string(),
            created: "2026-08-21T00:00:00Z".to_string(),
            requested: Requested {
                devices: devices.clone(),
                tracks: vec![],
                rate: 48000,
                bit_depth: "24".to_string(),
                midi: vec![],
                duration_s: None,
            },
            captured: Captured {
                devices,
                rate: 48000,
                bit_depth: "24".to_string(),
                channels: 0,
                frames: 0,
                duration_s: 0.0,
                xruns: total,
                xruns_by_device,
                audio_t0_monotonic_ns: 0,
                input_latency_frames: 0,
                midi_anchor_ns: 0,
                midi_events: 0,
                av_offset_ms: 0.0,
            },
            tracks: vec![],
            midi: None,
            verification: Verification {
                verified: false,
                checks: vec![],
            },
        }
    }

    #[test]
    fn single_device_clean_take_has_one_device_no_xruns_check_and_passes_it() {
        let m = manifest_with_xruns(vec![DeviceXruns {
            device: "Scarlett 18i20".to_string(),
            xruns: 0,
        }]);
        let v = run(Path::new("/nonexistent"), &m);
        let device_checks: Vec<&Check> = v
            .checks
            .iter()
            .filter(|c| c.name == "device_no_xruns")
            .collect();
        assert_eq!(device_checks.len(), 1);
        assert!(device_checks[0].ok);
    }

    #[test]
    fn multi_device_xrun_on_one_device_is_individually_attributable() {
        let m = manifest_with_xruns(vec![
            DeviceXruns {
                device: "BlackHole 2ch".to_string(),
                xruns: 0,
            },
            DeviceXruns {
                device: "BlackHole 16ch".to_string(),
                xruns: 3,
            },
        ]);
        let v = run(Path::new("/nonexistent"), &m);

        let device_checks: Vec<&Check> = v
            .checks
            .iter()
            .filter(|c| c.name == "device_no_xruns")
            .collect();
        assert_eq!(device_checks.len(), 2);

        let clean = device_checks
            .iter()
            .find(|c| c.detail.is_none())
            .expect("the clean device has no detail");
        assert!(clean.ok);

        let bad = device_checks
            .iter()
            .find(|c| c.detail.is_some())
            .expect("the glitched device has a detail string");
        assert!(!bad.ok);
        assert!(bad.detail.as_ref().unwrap().contains("BlackHole 16ch"));
        assert!(bad.detail.as_ref().unwrap().contains('3'));

        // The whole-take aggregate check also fails (sum > 0) — attribution is
        // additive, it doesn't replace the existing gate.
        let aggregate = v.checks.iter().find(|c| c.name == "no_xruns").unwrap();
        assert!(!aggregate.ok);

        assert!(!v.verified);
    }

    #[test]
    fn multi_device_all_clean_verifies_true_for_the_xrun_dimension() {
        let m = manifest_with_xruns(vec![
            DeviceXruns {
                device: "BlackHole 2ch".to_string(),
                xruns: 0,
            },
            DeviceXruns {
                device: "BlackHole 16ch".to_string(),
                xruns: 0,
            },
        ]);
        let v = run(Path::new("/nonexistent"), &m);
        let no_xrun_checks_ok = v
            .checks
            .iter()
            .filter(|c| c.name == "no_xruns" || c.name == "device_no_xruns")
            .all(|c| c.ok);
        assert!(no_xrun_checks_ok);
    }

    #[test]
    fn per_device_file_checks_attribute_a_missing_track_to_its_device() {
        let mut m = manifest_with_xruns(vec![DeviceXruns {
            device: "BlackHole 2ch".to_string(),
            xruns: 0,
        }]);
        m.requested.tracks = vec![RequestedTrack {
            name: "mic".to_string(),
            device: "BlackHole 2ch".to_string(),
            channels: vec![1, 2],
        }];
        m.tracks = vec![TrackInfo {
            file: "mic.wav".to_string(), // never written to disk in this test
            device: "BlackHole 2ch".to_string(),
            channels: vec![1, 2],
            peak_dbfs: 0.0,
            rms_dbfs: 0.0,
        }];
        let v = run(Path::new("/nonexistent-take-dir"), &m);
        let dev_exists = v
            .checks
            .iter()
            .find(|c| c.name == "device_files_exist_nonempty")
            .expect("one per-device exists check");
        assert!(!dev_exists.ok);
        assert!(dev_exists
            .detail
            .as_ref()
            .unwrap()
            .contains("BlackHole 2ch"));
        assert!(!v.verified);
    }

    #[test]
    fn midi_absent_is_not_gated() {
        let m = manifest_with_xruns(vec![DeviceXruns {
            device: "d".to_string(),
            xruns: 0,
        }]);
        let v = run(Path::new("/nonexistent"), &m);
        let av = v
            .checks
            .iter()
            .find(|c| c.name == "av_offset_within_tol")
            .unwrap();
        assert!(av.ok);
        assert!(!av.gating);
        let _ = MidiInfo {
            file: String::new(),
            ports: vec![],
            events: 0,
            note_ons: 0,
            note_offs: 0,
            synthesized_note_offs: 0,
        }; // referenced only to keep the import honest if MidiInfo is unused elsewhere
    }
}
