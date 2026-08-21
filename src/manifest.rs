//! `take.json` — the self-describing manifest for a take.
//!
//! The manifest records *what was requested*, *what was actually captured*, and
//! *the verification result*. `verified: true` (and exit 0) is the only trust
//! signal a downstream consumer should rely on.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Manifest schema version. Bump when the shape changes.
/// v2: added `captured.midi_anchor_ns` + `midi.synthesized_note_offs`, and
/// `av_offset_ms` now reports the MIDI→audio alignment shift applied.
/// v3: multi-device capture. `requested.device`/`captured.device` (singular)
/// became `devices` (plural); `RequestedTrack`/`TrackInfo` each gained a
/// `device` field attributing that track to its source device; `captured`
/// gained `xruns_by_device` so a multi-device take's verification can identify
/// which device failed instead of only reporting one whole-take xrun count.
/// All-devices-equal-one is still a fully valid (and the most common) shape.
pub const SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub tool_version: String,
    pub take_id: String,
    /// ISO-8601 UTC creation timestamp.
    pub created: String,
    pub requested: Requested,
    pub captured: Captured,
    pub tracks: Vec<TrackInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub midi: Option<MidiInfo>,
    pub verification: Verification,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requested {
    /// Device query strings as given (name, substring, or "default") — one per
    /// device. A single-device take is `devices.len() == 1`.
    pub devices: Vec<String>,
    pub tracks: Vec<RequestedTrack>,
    /// Shared across every device in the take — see `record::resolve_plan`'s
    /// same-rate-across-devices validation (no cross-device resampling).
    pub rate: u32,
    /// "16" | "24" | "32f".
    pub bit_depth: String,
    pub midi: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestedTrack {
    pub name: String,
    /// Which requested device (matches one entry in `Requested.devices`) this
    /// track's channels are relative to.
    pub device: String,
    /// 1-based, relative to `device`'s own channel numbering.
    pub channels: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Captured {
    /// Resolved concrete device names (a "default"/substring query resolves to
    /// one real name) — one per device, same order as `Requested.devices`.
    pub devices: Vec<String>,
    pub rate: u32,
    pub bit_depth: String,
    /// Total channels captured across all tracks (all devices combined).
    pub channels: u16,
    /// Frames captured, as the MINIMUM across every track on every device (the
    /// same "shortest track defines the take" rule v0.2 already applied within
    /// one device, now applied across all of them).
    pub frames: u64,
    pub duration_s: f64,
    /// Total dropped-frame / overflow events summed across all devices. The
    /// heart of the contract: this must be zero for a take to verify. See
    /// `xruns_by_device` for which device(s) contributed, and
    /// `record::DropMonitor`.
    pub xruns: u64,
    /// Per-device breakdown of the count above, so a multi-device take's
    /// verification can name which device glitched rather than only failing
    /// the take as a whole. Always has one entry per `devices` entry, even for
    /// a single-device take (a one-element vec).
    pub xruns_by_device: Vec<DeviceXruns>,
    /// From the take's first/primary device (`devices[0]`) — the shared
    /// `SessionClock` means every device's t0 is comparable, but only the
    /// primary device's audio_t0/latency feed the MIDI anchor calculation
    /// (MIDI is a single armed input, not per-device).
    pub audio_t0_monotonic_ns: u128,
    pub input_latency_frames: u64,
    /// Nanoseconds subtracted from every MIDI event so the MIDI timeline shares
    /// its zero with audio sample 0 (= `audio_t0 − input_latency`, primary
    /// device).
    pub midi_anchor_ns: u128,
    pub midi_events: u64,
    /// The MIDI→audio alignment shift applied, in ms (= `midi_anchor_ns` / 1e6).
    /// The residual after this compensation is MIDI transport jitter + the
    /// instrument's own note latency; gated honestly by the loopback fixture.
    pub av_offset_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceXruns {
    pub device: String,
    pub xruns: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    pub file: String,
    /// Resolved concrete device name this track's audio came from (matches one
    /// entry in `Captured.devices`).
    pub device: String,
    pub channels: Vec<u16>,
    pub peak_dbfs: f32,
    pub rms_dbfs: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidiInfo {
    pub file: String,
    pub ports: Vec<String>,
    pub events: u64,
    pub note_ons: u64,
    pub note_offs: u64,
    /// Note-offs synthesized to close notes still held when capture stopped.
    pub synthesized_note_offs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verification {
    pub verified: bool,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    /// Whether failing this check fails the take. Non-gating checks are reported
    /// but do not (yet) block — used for metrics we cannot honestly gate on in
    /// v0.2 (e.g. A/V offset, which needs the loopback fixture to calibrate).
    pub gating: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Check {
    pub fn gating(name: &str, ok: bool, detail: Option<String>) -> Self {
        Check {
            name: name.to_string(),
            ok,
            gating: true,
            detail,
        }
    }

    pub fn info(name: &str, ok: bool, detail: Option<String>) -> Self {
        Check {
            name: name.to_string(),
            ok,
            gating: false,
            detail,
        }
    }
}

/// Deterministic take id: `rec-<8 hex>` from a content hash of the requested
/// config plus the creation timestamp. Same inputs ⇒ same id (echoing the
/// Workchain SHA-256 content-hash convention).
pub fn take_id(requested: &Requested, created: &str) -> String {
    let mut h = Sha256::new();
    let canon = serde_json::to_string(requested).unwrap_or_default();
    h.update(canon.as_bytes());
    h.update(b"\x01");
    h.update(created.as_bytes());
    let d = h.finalize();
    format!("rec-{:02x}{:02x}{:02x}{:02x}", d[0], d[1], d[2], d[3])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> Requested {
        Requested {
            devices: vec!["Scarlett 18i20".into()],
            tracks: vec![RequestedTrack {
                name: "mic".into(),
                device: "Scarlett 18i20".into(),
                channels: vec![1, 2],
            }],
            rate: 48000,
            bit_depth: "24".into(),
            midi: vec![],
            duration_s: None,
        }
    }

    #[test]
    fn take_id_is_deterministic_and_prefixed() {
        let r = req();
        let a = take_id(&r, "2026-07-18T15:30:00Z");
        let b = take_id(&r, "2026-07-18T15:30:00Z");
        assert_eq!(a, b);
        assert!(a.starts_with("rec-"));
        assert_eq!(a.len(), "rec-".len() + 8);
    }

    #[test]
    fn take_id_changes_with_time() {
        let r = req();
        let a = take_id(&r, "2026-07-18T15:30:00Z");
        let b = take_id(&r, "2026-07-18T15:30:01Z");
        assert_ne!(a, b);
    }

    #[test]
    fn multi_device_request_serializes_with_per_track_device() {
        let r = Requested {
            devices: vec!["BlackHole 2ch".into(), "BlackHole 16ch".into()],
            tracks: vec![
                RequestedTrack {
                    name: "mic".into(),
                    device: "BlackHole 2ch".into(),
                    channels: vec![1, 2],
                },
                RequestedTrack {
                    name: "call".into(),
                    device: "BlackHole 16ch".into(),
                    channels: vec![1, 2],
                },
            ],
            rate: 48000,
            bit_depth: "24".into(),
            midi: vec![],
            duration_s: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["devices"].as_array().unwrap().len(), 2);
        assert_eq!(v["tracks"][0]["device"], "BlackHole 2ch");
        assert_eq!(v["tracks"][1]["device"], "BlackHole 16ch");
    }
}
