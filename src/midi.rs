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
use midly::num::{u15, u24, u28};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};

/// SMF timing: 480 ticks/quarter at a nominal 120 BPM, so 960 ticks/second.
/// A recorder captures absolute time; the metrical framing is a DAW-friendly
/// default (the wall-clock spacing of events is preserved at 120 BPM).
const TPQN: u16 = 480;
const TICKS_PER_SECOND: f64 = TPQN as f64 * 2.0;
const DEFAULT_TEMPO_US_PER_QN: u32 = 500_000; // 120 BPM

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

    let probe = MidiInput::new("lufs-recorder-probe").map_err(|e| e.to_string())?;
    let want_all = spec.eq_ignore_ascii_case("all");
    let needle = spec.to_lowercase();

    let mut selected = Vec::new();
    for port in probe.ports() {
        let name = probe.port_name(&port).unwrap_or_default();
        if want_all || name.to_lowercase().contains(&needle) {
            selected.push((port, name));
        }
    }
    if selected.is_empty() {
        return Err(format!("MIDI input port matching '{spec}' not found"));
    }
    drop(probe);

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
    pub events: u64,
    pub note_ons: u64,
    pub note_offs: u64,
}

/// Write captured events to a Standard MIDI File and return the counts.
pub fn write_smf(path: &Path, events: &[RawMidiEvent]) -> Result<MidiCounts> {
    let mut track: Track = Vec::new();
    track.push(TrackEvent {
        delta: u28::from_int_lossy(0),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::from_int_lossy(
            DEFAULT_TEMPO_US_PER_QN,
        ))),
    });

    let mut counts = MidiCounts::default();
    let mut prev_tick: u64 = 0;

    for ev in events {
        let parsed = match LiveEvent::parse(&ev.data) {
            Ok(p) => p,
            Err(_) => continue, // skip malformed / partial messages
        };
        if let LiveEvent::Midi { channel, message } = parsed {
            match message {
                MidiMessage::NoteOn { vel, .. } => {
                    if vel.as_int() > 0 {
                        counts.note_ons += 1;
                    } else {
                        counts.note_offs += 1; // note-on vel 0 == note-off
                    }
                }
                MidiMessage::NoteOff { .. } => counts.note_offs += 1,
                _ => {}
            }
            let tick = (ev.offset_ns as f64 / 1e9 * TICKS_PER_SECOND).round() as u64;
            let delta = tick.saturating_sub(prev_tick);
            prev_tick = tick;
            track.push(TrackEvent {
                delta: u28::from_int_lossy(delta.min(u32::MAX as u64) as u32),
                kind: TrackEventKind::Midi { channel, message },
            });
            counts.events += 1;
        }
    }

    track.push(TrackEvent {
        delta: u28::from_int_lossy(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

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

    #[test]
    fn smf_roundtrip_counts_notes() {
        let dir = std::env::temp_dir().join(format!("lufs-rec-miditest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("capture.mid");

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
        let counts = write_smf(&path, &events).unwrap();
        assert_eq!(counts.events, 2);
        assert_eq!(counts.note_ons, 1);
        assert_eq!(counts.note_offs, 1);
        assert!(path.exists());
        assert!(std::fs::metadata(&path).unwrap().len() > 0);

        // File parses back as a valid SMF.
        let bytes = std::fs::read(&path).unwrap();
        assert!(Smf::parse(&bytes).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
