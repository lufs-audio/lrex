//! The shared session timeline.
//!
//! One monotonic clock authority for the whole take. Audio and MIDI are both
//! stamped against the same `t0`, so their offsets are directly comparable — the
//! discipline that makes A/V alignment engineering rather than luck (see the
//! design suite, `02-research-and-landscape`).

use std::time::Instant;

/// A monotonic session clock. `t0` is captured once at record start; every
/// subsequent event is expressed as nanoseconds since `t0`.
#[derive(Debug, Clone, Copy)]
pub struct SessionClock {
    t0: Instant,
}

impl SessionClock {
    /// Start the session clock (call once, before opening any stream).
    pub fn start() -> Self {
        SessionClock { t0: Instant::now() }
    }

    /// Nanoseconds elapsed since `t0` at the moment of the call.
    pub fn now_ns(&self) -> u128 {
        self.t0.elapsed().as_nanos()
    }
}
