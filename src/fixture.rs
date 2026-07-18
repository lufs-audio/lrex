//! In-process self-test fixtures — the dependency-free "loopback".
//!
//! Instead of routing real audio/MIDI through a virtual device (BlackHole / IAC)
//! or hardware, we inject a *known* signal straight into the real pipeline and
//! assert we recover it: the audio de-interleave + 24-bit WAV round-trip (right
//! channel → right file), the MIDI SMF export (anchor + hanging-note closure),
//! and the A/V anchor math. This runs in CI and via `lufs-recorder selftest`, so
//! a build can prove the parts we own are correct with no audio hardware at all.
//! A true device loopback stays an optional manual check on a real machine.

use crate::manifest::Check;
use crate::midi::{self, RawMidiEvent};
use crate::record;
use anyhow::{Context, Result};
use std::path::Path;

const SAMPLE_RATE: u32 = 48_000;
/// Metrical ms-per-tick equivalent for timing assertions: 960 ticks/second.
const TICKS_PER_SECOND: f64 = 960.0;

pub struct FixtureReport {
    pub checks: Vec<Check>,
}

impl FixtureReport {
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| !c.gating || c.ok)
    }
}

/// Run every fixture and collect its checks.
pub fn run_all() -> Result<FixtureReport> {
    let dir = std::env::temp_dir().join(format!("lufs-recorder-selftest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).context("creating selftest dir")?;
    let mut checks = Vec::new();
    checks.extend(audio_roundtrip(&dir)?);
    checks.extend(midi_roundtrip(&dir)?);
    checks.extend(av_anchor_math());
    let _ = std::fs::remove_dir_all(&dir);
    Ok(FixtureReport { checks })
}

/// Read a WAV back as per-channel means (int samples normalized to f32 range).
fn wav_channel_means(path: &Path) -> Result<(u16, u64, Vec<f64>)> {
    let mut r =
        hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let mut sums = vec![0f64; ch.max(1)];
    let mut count = 0u64;

    match spec.sample_format {
        hound::SampleFormat::Float => {
            for (i, s) in r.samples::<f32>().enumerate() {
                sums[i % ch] += s? as f64;
                if i % ch == ch - 1 {
                    count += 1;
                }
            }
        }
        hound::SampleFormat::Int => {
            let denom = 2f64.powi(spec.bits_per_sample as i32 - 1);
            for (i, s) in r.samples::<i32>().enumerate() {
                sums[i % ch] += s? as f64 / denom;
                if i % ch == ch - 1 {
                    count += 1;
                }
            }
        }
    }

    let means = sums
        .iter()
        .map(|s| if count == 0 { 0.0 } else { s / count as f64 })
        .collect();
    Ok((spec.channels, count, means))
}

/// Inject a distinct DC level per device channel, de-interleave two stereo
/// tracks (mic ⟵ 1-2, piano ⟵ 9-10) through the real WAV write path, and assert
/// each channel lands in the right file at the right level (routing + 24-bit
/// round-trip).
fn audio_roundtrip(dir: &Path) -> Result<Vec<Check>> {
    const DEV: usize = 10;
    let frames: usize = (SAMPLE_RATE / 4) as usize; // 0.25 s

    // Channel c (0-based) carries DC = (c+1)/100 → ch1=0.01 … ch10=0.10.
    let value = |c: usize| -> f32 { (c as f32 + 1.0) * 0.01 };
    let mut interleaved = Vec::with_capacity(DEV * frames);
    for _ in 0..frames {
        for c in 0..DEV {
            interleaved.push(value(c));
        }
    }

    // The parity layout, expressed as (name, 1-based channels).
    let tracks: [(&str, [u16; 2]); 2] = [("mic", [1, 2]), ("piano", [9, 10])];

    let mut routing_ok = true;
    let mut worst_err = 0.0f64;
    let mut frames_ok = true;

    for (name, chans) in &tracks {
        let path = dir.join(format!("{name}.wav"));
        let mut w = hound::WavWriter::create(
            &path,
            record::wav_spec(chans.len() as u16, SAMPLE_RATE, "24"),
        )
        .with_context(|| format!("creating {}", path.display()))?;
        for f in 0..frames {
            for &ch in chans {
                let s = interleaved[f * DEV + (ch as usize - 1)];
                record::write_sample(&mut w, "24", s)?;
            }
        }
        w.finalize()?;

        let (_ch, n, means) = wav_channel_means(&path)?;
        if n as usize != frames {
            frames_ok = false;
        }
        for (i, &ch) in chans.iter().enumerate() {
            let expected = value(ch as usize - 1) as f64;
            let err = (means[i] - expected).abs();
            worst_err = worst_err.max(err);
            if err > 1e-3 {
                routing_ok = false;
            }
        }
    }

    Ok(vec![
        Check::gating(
            "audio_routing",
            routing_ok,
            (!routing_ok).then(|| {
                format!("a channel landed in the wrong file or level (worst err {worst_err:.6})")
            }),
        ),
        Check::gating(
            "audio_24bit_roundtrip",
            worst_err <= 1e-3,
            (worst_err > 1e-3).then(|| format!("24-bit round-trip error {worst_err:.6} > 1e-3")),
        ),
        Check::gating(
            "audio_frame_count",
            frames_ok,
            (!frames_ok).then(|| format!("recovered frame count != {frames}")),
        ),
    ])
}

/// Inject known MIDI (including a held note with no explicit off), export with a
/// non-zero anchor, and assert balance, hanging-note closure, valid SMF, and
/// that the anchor shifted the first note to the expected tick.
fn midi_roundtrip(dir: &Path) -> Result<Vec<Check>> {
    let anchor_ns: u128 = 50_000_000; // 50 ms
    let end_ns: u128 = 1_000_000_000; // 1 s
    let events = vec![
        RawMidiEvent {
            offset_ns: 100_000_000,
            data: vec![0x90, 60, 100],
        }, // on @100ms
        RawMidiEvent {
            offset_ns: 600_000_000,
            data: vec![0x80, 60, 0],
        }, // off @600ms
        RawMidiEvent {
            offset_ns: 700_000_000,
            data: vec![0x90, 64, 90],
        }, // on @700ms, HELD (no off)
    ];

    let path = dir.join("selftest.mid");
    let counts = midi::write_smf(&path, &events, anchor_ns, end_ns)?;

    let balanced = counts.note_ons == counts.note_offs;
    let hanging_closed = counts.synthesized_offs == 1;

    // Parse back: valid SMF, count note-ons, and the first note-on's tick.
    let bytes = std::fs::read(&path)?;
    let mut smf_ok = false;
    let mut file_note_ons = 0u64;
    let mut first_on_tick: Option<u64> = None;
    if let Ok(smf) = midly::Smf::parse(&bytes) {
        smf_ok = true;
        if let Some(track) = smf.tracks.first() {
            let mut acc: u64 = 0;
            for ev in track {
                acc += ev.delta.as_int() as u64;
                if let midly::TrackEventKind::Midi {
                    message: midly::MidiMessage::NoteOn { vel, .. },
                    ..
                } = ev.kind
                {
                    if vel.as_int() > 0 {
                        file_note_ons += 1;
                        first_on_tick.get_or_insert(acc);
                    }
                }
            }
        }
    }

    // First note-on: 100ms − 50ms anchor = 50ms → 0.05 * 960 = 48 ticks.
    let expected_tick =
        ((100_000_000u128 - anchor_ns) as f64 / 1e9 * TICKS_PER_SECOND).round() as u64;
    let anchor_timing_ok = first_on_tick
        .map(|t| t.abs_diff(expected_tick) <= 1)
        .unwrap_or(false);

    Ok(vec![
        Check::gating(
            "midi_balance",
            balanced,
            (!balanced).then(|| {
                format!(
                    "note-ons {} != note-offs {}",
                    counts.note_ons, counts.note_offs
                )
            }),
        ),
        Check::gating(
            "midi_hanging_note_closed",
            hanging_closed,
            (!hanging_closed).then(|| {
                format!(
                    "expected 1 synthesized off, got {}",
                    counts.synthesized_offs
                )
            }),
        ),
        Check::gating(
            "midi_smf_parses",
            smf_ok && file_note_ons == 2,
            (!(smf_ok && file_note_ons == 2))
                .then(|| format!("smf_ok={smf_ok}, note-ons in file={file_note_ons} (want 2)")),
        ),
        Check::gating(
            "midi_anchor_timing",
            anchor_timing_ok,
            (!anchor_timing_ok).then(|| {
                format!("first note-on tick {first_on_tick:?} != expected {expected_tick} (±1)")
            }),
        ),
    ])
}

/// Assert the MIDI→audio anchor arithmetic (`audio_t0 − input_latency`) that
/// `record` applies to every take.
fn av_anchor_math() -> Vec<Check> {
    // 105 ms audio_t0, 511 frames latency @ 48k ≈ 10.645 ms → anchor ≈ 94.354 ms.
    let anchor = record::compute_anchor_ns(105_000_000, 511, SAMPLE_RATE);
    let latency_ns = 511u128 * 1_000_000_000 / SAMPLE_RATE as u128;
    let expected = 105_000_000u128 - latency_ns;
    let ok = anchor == expected;
    vec![Check::gating(
        "av_anchor_math",
        ok,
        (!ok).then(|| format!("anchor {anchor} != expected {expected}")),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_fixtures_pass() {
        let report = run_all().unwrap();
        let failures: Vec<&Check> = report.checks.iter().filter(|c| !c.ok && c.gating).collect();
        assert!(report.passed(), "selftest failures: {failures:?}");
        // Sanity: we actually ran a meaningful number of checks.
        assert!(report.checks.len() >= 8);
    }
}
