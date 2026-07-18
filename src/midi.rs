//! MIDI capture (midir) + Standard MIDI File export (midly).
//!
//! Timeline discipline (see design suite `02-research-and-landscape`): we do
//! **not** trust the backend's delta-time. Every incoming message is stamped
//! with our own monotonic [`SessionClock`] at the callback boundary; SMF delta
//! ticks are computed only at export.

use crate::clock::SessionClock;
use anyhow::{Context, Result};
use midir::{MidiInput, MidiInputConnection};
use midly::live::LiveEvent;
use midly::num::{u15, u24, u28, u4, u7};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};

/// Metrical SMF timing: 480 ticks/quarter at 120 BPM (500000 µs/qn) => 960
/// ticks/second. This is a standard, universally-importable MIDI file; a DAW
/// conforms it to its own project tempo — that tempo-relative rescaling is the
/// expected metrical-MIDI behavior (you align the take like any recorded
/// region). We tried SMPTE absolute timing to lock wall-clock time, but not all
/// DAWs import it, so we keep metrical for compatibility.
const TPQN: u16 = 480;
const TICKS_PER_SECOND: f64 = TPQN as f64 * 2.0; // 120 BPM
const DEFAULT_TEMPO_US_PER_QN: u32 = 500_000; // 120 BPM

/// Convert nanoseconds (relative to the MIDI anchor) to metrical ticks.
fn tick_of(rel_ns: u128) -> u64 {
    ((rel_ns as f64) / 1e9 * TICKS_PER_SECOND).round() as u64
}

/// A raw MIDI message stamped against the session clock.
pub struct RawMidiEvent {
    pub offset_ns: u128,
    pub data: Vec<u8>,
}

/// An armed MIDI capture. Dropping it (or calling [`MidiCapture::finish`])
/// disconnects the ports and stops the callbacks.
pub struct MidiCapture {
    conns: Vec<MidiInputConnection<()>>,
    rx: Receiver<RawMidiEvent>,
    pub ports: Vec<String>,
}

/// Whether a port `name` matches a MIDI spec: `"all"` matches everything;
/// otherwise the spec is a comma-separated list of case-insensitive substrings,
/// any of which matching counts (e.g. `"Nord,Syntakt"`).
pub fn port_matches(name: &str, spec: &str) -> bool {
    if spec.eq_ignore_ascii_case("all") {
        return true;
    }
    let lname = name.to_lowercase();
    spec.split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .any(|needle| lname.contains(&needle))
}

/// List available MIDI input port names.
pub fn list_ports() -> Result<Vec<String>> {
    let input = MidiInput::new("lufs-recorder-list").context("opening MIDI input")?;
    Ok(input
        .ports()
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .collect())
}

/// Arm MIDI capture. `spec` is a port name/substring, "all", or "off".
/// Returns `Ok(None)` for "off"; a human message (mapped to exit 3) if a
/// requested port can't be found.
pub fn arm(spec: &str, clock: SessionClock) -> std::result::Result<Option<MidiCapture>, String> {
    if spec.eq_ignore_ascii_case("off") {
        return Ok(None);
    }

    // Enumerate with a short-lived probe input, released by RAII (scope end)
    // before we open the capture connections. Not an explicit `drop()`: on the
    // CoreAudio backend `MidiInput` doesn't implement `Drop`, so dropping it is
    // a no-op that only extends its lifetime (clippy::drop_non_drop).
    let selected: Vec<(midir::MidiInputPort, String)> = {
        let probe = MidiInput::new("lufs-recorder-probe").map_err(|e| e.to_string())?;
        probe
            .ports()
            .into_iter()
            .map(|port| {
                let name = probe.port_name(&port).unwrap_or_default();
                (port, name)
            })
            .filter(|(_, name)| port_matches(name, spec))
            .collect()
    };
    if selected.is_empty() {
        return Err(format!("MIDI input port matching '{spec}' not found"));
    }

    let (tx, rx) = channel::<RawMidiEvent>();
    let mut conns = Vec::new();
    let mut ports = Vec::new();

    for (port, name) in selected {
        let input = MidiInput::new("lufs-recorder-in").map_err(|e| e.to_string())?;
        let txc = tx.clone();
        let clk = clock;
        let conn = input
            .connect(
                &port,
                "lufs-recorder-in",
                move |_stamp, message, _| {
                    // Ignore midir's own (unreliable) timestamp; use our clock.
                    let _ = txc.send(RawMidiEvent {
                        offset_ns: clk.now_ns(),
                        data: message.to_vec(),
                    });
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        conns.push(conn);
        ports.push(name);
    }

    Ok(Some(MidiCapture { conns, rx, ports }))
}

impl MidiCapture {
    /// Stop capture and return all buffered events, ordered by time.
    pub fn finish(self) -> Vec<RawMidiEvent> {
        let MidiCapture { conns, rx, .. } = self;
        // Disconnecting drops the callback senders; the channel then closes.
        for c in conns {
            c.close();
        }
        let mut events: Vec<RawMidiEvent> = rx.try_iter().collect();
        events.sort_by_key(|e| e.offset_ns);
        events
    }
}

/// Counts derived from a captured MIDI stream.
#[derive(Debug, Clone, Copy, Default)]
pub struct MidiCounts {
    /// Channel-voice messages actually captured (excludes synthesized offs).
    pub events: u64,
    pub note_ons: u64,
    /// Explicit note-offs captured **plus** synthesized end-of-take offs.
    pub note_offs: u64,
    /// Note-offs synthesized for notes still held when capture stopped.
    pub synthesized_offs: u64,
}

/// Append a track event, computing its delta from `prev_tick`.
fn push_event<'a>(
    track: &mut Vec<TrackEvent<'a>>,
    prev_tick: &mut u64,
    tick: u64,
    kind: TrackEventKind<'a>,
) {
    let delta = tick.saturating_sub(*prev_tick);
    *prev_tick = tick;
    track.push(TrackEvent {
        delta: u28::from_int_lossy(delta.min(u32::MAX as u64) as u32),
        kind,
    });
}

/// Write captured events to a Standard MIDI File and return the counts.
///
/// `anchor_ns` is subtracted from every event's session-relative offset so the
/// MIDI timeline shares its zero with audio sample 0 (pass `audio_t0 −
/// input_latency`); this is the input-latency compensation that aligns MIDI to
/// what you hear. `end_ns` is the take end (session-relative) where note-offs
/// are synthesized for any notes still held when capture stopped — DAW punch-out
/// behavior, so the file is valid and note-ons balance note-offs.
pub fn write_smf(
    path: &Path,
    events: &[RawMidiEvent],
    anchor_ns: u128,
    end_ns: u128,
) -> Result<MidiCounts> {
    let mut track: Track = Vec::new();
    let mut counts = MidiCounts::default();
    let mut prev_tick: u64 = 0;
    // Notes currently sounding, so we can close any still held at capture end.
    let mut held: BTreeSet<(u8, u8)> = BTreeSet::new();

    // Tempo meta at t=0 (120 BPM). Metrical timing is tempo-relative by design.
    push_event(
        &mut track,
        &mut prev_tick,
        0,
        TrackEventKind::Meta(MetaMessage::Tempo(u24::from_int_lossy(
            DEFAULT_TEMPO_US_PER_QN,
        ))),
    );

    for ev in events {
        let parsed = match LiveEvent::parse(&ev.data) {
            Ok(p) => p,
            Err(_) => continue, // skip malformed / partial messages
        };
        if let LiveEvent::Midi { channel, message } = parsed {
            match message {
                MidiMessage::NoteOn { key, vel } => {
                    if vel.as_int() > 0 {
                        counts.note_ons += 1;
                        held.insert((channel.as_int(), key.as_int()));
                    } else {
                        counts.note_offs += 1; // note-on vel 0 == note-off
                        held.remove(&(channel.as_int(), key.as_int()));
                    }
                }
                MidiMessage::NoteOff { key, .. } => {
                    counts.note_offs += 1;
                    held.remove(&(channel.as_int(), key.as_int()));
                }
                _ => {}
            }
            let tick = tick_of(ev.offset_ns.saturating_sub(anchor_ns));
            push_event(
                &mut track,
                &mut prev_tick,
                tick,
                TrackEventKind::Midi { channel, message },
            );
            counts.events += 1;
        }
    }

    // Close notes still held at capture end (never before their note-on).
    let end_tick = tick_of(end_ns.saturating_sub(anchor_ns)).max(prev_tick);
    for (channel, key) in held.iter().copied() {
        let kind = TrackEventKind::Midi {
            channel: u4::from_int_lossy(channel),
            message: MidiMessage::NoteOff {
                key: u7::from_int_lossy(key),
                vel: u7::from_int_lossy(0),
            },
        };
        push_event(&mut track, &mut prev_tick, end_tick, kind);
        counts.note_offs += 1;
        counts.synthesized_offs += 1;
    }

    let last_tick = prev_tick;
    push_event(
        &mut track,
        &mut prev_tick,
        last_tick,
        TrackEventKind::Meta(MetaMessage::EndOfTrack),
    );

    let smf = Smf {
        header: Header::new(
            Format::SingleTrack,
            Timing::Metrical(u15::from_int_lossy(TPQN)),
        ),
        tracks: vec![track],
    };
    smf.save(path)
        .with_context(|| format!("writing MIDI file {}", path.display()))?;

    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lufs-rec-miditest-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("capture.mid")
    }

    #[test]
    fn smf_roundtrip_counts_notes() {
        let path = tmp("balanced");
        // note-on then note-off, ~0.5s apart, on channel 1.
        let events = vec![
            RawMidiEvent {
                offset_ns: 0,
                data: vec![0x90, 60, 100],
            },
            RawMidiEvent {
                offset_ns: 500_000_000,
                data: vec![0x80, 60, 0],
            },
        ];
        let counts = write_smf(&path, &events, 0, 1_000_000_000).unwrap();
        assert_eq!(counts.events, 2);
        assert_eq!(counts.note_ons, 1);
        assert_eq!(counts.note_offs, 1);
        assert_eq!(counts.synthesized_offs, 0);
        assert!(std::fs::metadata(&path).unwrap().len() > 0);

        // Parses back, and the timing division is metrical (universally importable).
        let bytes = std::fs::read(&path).unwrap();
        let smf = Smf::parse(&bytes).expect("valid SMF");
        assert!(matches!(smf.header.timing, Timing::Metrical(..)));

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn held_note_is_closed_at_capture_end() {
        let path = tmp("hanging");
        // A note-on with no matching note-off (key held when capture stopped).
        let events = vec![RawMidiEvent {
            offset_ns: 200_000_000,
            data: vec![0x90, 64, 100],
        }];
        let counts = write_smf(&path, &events, 0, 5_000_000_000).unwrap();
        assert_eq!(counts.note_ons, 1);
        assert_eq!(
            counts.note_offs, 1,
            "held note must be balanced by a synth off"
        );
        assert_eq!(counts.synthesized_offs, 1);

        let bytes = std::fs::read(&path).unwrap();
        assert!(Smf::parse(&bytes).is_ok());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn port_matching() {
        assert!(port_matches("Nord Stage 3 MIDI Output", "Nord Stage 3"));
        assert!(port_matches("Elektron Syntakt", "nord,syntakt"));
        assert!(port_matches("anything", "all"));
        assert!(!port_matches("IAC Driver Bus 1", "nord,syntakt"));
        assert!(!port_matches("IAC Driver Bus 1", ""));
    }

    #[test]
    fn anchor_shifts_events_toward_zero() {
        // An event at 300ms with a 100ms anchor lands at ~200ms (tick 200).
        let path = tmp("anchor");
        let events = vec![
            RawMidiEvent {
                offset_ns: 300_000_000,
                data: vec![0x90, 60, 100],
            },
            RawMidiEvent {
                offset_ns: 800_000_000,
                data: vec![0x80, 60, 0],
            },
        ];
        let counts = write_smf(&path, &events, 100_000_000, 1_000_000_000).unwrap();
        assert_eq!(counts.note_ons, 1);
        assert_eq!(counts.note_offs, 1);
        let bytes = std::fs::read(&path).unwrap();
        assert!(Smf::parse(&bytes).is_ok());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
