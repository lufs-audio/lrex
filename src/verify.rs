//! The verification contract — turns "a take ran" into "a take is proven".
//!
//! Re-derives the verification result from the files on disk versus the claims
//! in the manifest. Used both inline after `record` and standalone via
//! `verify <take-dir>`. `verified` is true only when every *gating* check passes.

use crate::manifest::{Check, Manifest, Verification};
use anyhow::{Context, Result};
use std::path::Path;

/// Duration tolerance: the larger of 0.5s or 5% of the expected length.
fn duration_tolerance(expected_s: f64) -> f64 {
    (expected_s * 0.05).max(0.5)
}

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

    for (track, req) in manifest.tracks.iter().zip(manifest.requested.tracks.iter()) {
        let path = take_dir.join(&track.file);
        let nonempty = std::fs::metadata(&path)
            .map(|m| m.len() > 44)
            .unwrap_or(false);
        if !nonempty {
            all_exist = false;
            continue;
        }
        match read_wav_stats(&path) {
            Ok(st) => {
                if st.channels as usize != req.channels.len() {
                    channels_match = false;
                }
                if st.sample_rate != manifest.captured.rate {
                    rate_match = false;
                }
                if st.bits_per_sample != want_bits || st.is_float != want_float {
                    depth_match = false;
                }
                match total_frames {
                    None => total_frames = Some(st.frames),
                    Some(f) if f != st.frames => total_frames = Some(f.min(st.frames)),
                    _ => {}
                }
                min_peak_dbfs = min_peak_dbfs.min(st.peak_dbfs);
                max_peak_dbfs = max_peak_dbfs.max(st.peak_dbfs);
            }
            Err(_) => all_decode = false,
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

    // --- The heart of the contract: no dropped frames ---
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

    // --- A/V offset: reported, not gated in v0.2 (needs loopback calibration) ---
    checks.push(Check::info(
        "av_offset_within_tol",
        true,
        Some(format!(
            "{:.2} ms (informational)",
            manifest.captured.av_offset_ms
        )),
    ));

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
