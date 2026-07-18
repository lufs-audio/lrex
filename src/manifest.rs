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
pub const SCHEMA_VERSION: u32 = 2;

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
    pub device: String,
    pub tracks: Vec<RequestedTrack>,
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
    /// 1-based device input channels captured into this track.
    pub channels: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Captured {
    pub device: String,
    pub rate: u32,
    pub bit_depth: String,
    /// Total channels captured across all tracks.
    pub channels: u16,
    /// Frames captured per track (all tracks share the clock, so all equal).
    pub frames: u64,
    pub duration_s: f64,
    /// Estimated dropped-frame / overflow events. The heart of the contract:
    /// this must be zero for a take to verify. See `record::DropMonitor`.
    pub xruns: u64,
    pub audio_t0_monotonic_ns: u128,
    pub input_latency_frames: u64,
    /// Nanoseconds subtracted from every MIDI event so the MIDI timeline shares
    /// its zero with audio sample 0 (= `audio_t0 − input_latency`).
    pub midi_anchor_ns: u128,
    pub midi_events: u64,
    /// The MIDI→audio alignment shift applied, in ms (= `midi_anchor_ns` / 1e6).
    /// The residual after this compensation is MIDI transport jitter + the
    /// instrument's own note latency; gated honestly by the loopback fixture.
    pub av_offset_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    pub file: String,
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
            device: "Scarlett 18i20".into(),
            tracks: vec![RequestedTrack {
                name: "mic".into(),
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
}
