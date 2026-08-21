# Unit 01 — Multi-Device Capture Engine

*Status: implemented (this PR). See SPEC.md §5 for where the design resolved
open questions during implementation.*

## Objective
Replace `RecordPlan`'s single `device_query: String` with support for capturing
from multiple audio devices concurrently in one take, with each device's
channels correctly attributed to their source in `take.json`.

## Context
- Proposal: `docs/proposals/01-multi-device-and-voice-call-profiles.md`
- SPEC.md (this suite), §4-5
- Code: `src/record.rs` (`RecordPlan`, `resolve_plan`, `resolve_devices`, `run`,
  `dry_run`), `src/cli.rs` (`RecordArgs::device_track`)
- Must keep honoring: `CONTRACT.md`'s per-take deterministic assertions
  (channel count, rate/bit-depth match, zero xruns)

## Acceptance criteria
- [x] `RecordPlan` gained device plurality: `devices: Vec<DeviceCapture>`, each
      `DeviceCapture { device_query, tracks }` — resolved from the hedge in the
      original draft (see SPEC.md §5).
- [x] The capture loop opens and reads from N devices concurrently within one
      take: each device gets its own ring buffer, writer thread, and xrun
      counter (not sequential captures stitched together), all sharing one
      `SessionClock` so cross-device and audio/MIDI offsets stay comparable.
- [x] Every `tracks[]` entry in `take.json` is attributed to its source device
      via a new `device` field (see Unit 02 / manifest schema v3).
- [x] A 2-device take produces exactly one `take.json`, one directory, one
      take-id (structural — the take directory and manifest are created once,
      before the per-device loop, and every device writes into it).
- [x] Existing callers passing a single device see unchanged output shape —
      verified by `resolve_plan_legacy_single_device_is_unchanged` and by the
      untouched `--device`/`--track`/`--channels` code path.
- [x] `resolve_plan()` exposes a stable seam (`crate::config::resolve_profile_duration_secs`)
      that Unit 03 supplies `duration` through, without Unit 01 owning the
      Config profile schema.
- [x] Every device in a take must resolve to the same sample rate — no
      cross-device resampling. `resolve_devices()` fails closed with
      `FormatUnsupported` (exit 4) on a mismatch, naming both devices and both
      rates in the error.

## Interface contract
```rust
struct DeviceCapture {
    device_query: String,
    tracks: Vec<(String, Vec<u16>)>,
}

struct RecordPlan {
    devices: Vec<DeviceCapture>,   // was: device_query: String, tracks: Vec<...>
    want_rate: Option<u32>,
    bit_depth: String,
    midi_spec: String,
    out_dir: PathBuf,
    name: Option<String>,
    duration: Option<f64>,
}

fn resolve_plan(cfg: &Config, args: &cli::RecordArgs) -> crate::error::Result<RecordPlan>
fn resolve_devices(plan: &RecordPlan) -> crate::error::Result<Vec<ResolvedDevice>>
```
CLI surface: `--device-track DEVICE:NAME=CHANNELS` (repeatable; grouped by
device in first-seen order), `conflicts_with_all = ["device", "track", "channels"]`.
Legacy `--device`/`--track`/`--channels` are untouched and remain the
single-device path (now internally represented as a one-element `devices` vec).

## Boundaries — do NOT touch
- Config schema / profiles (Unit 03 owns `Config`'s new fields and
  `resolve_profile_duration_secs`; Unit 01 only calls it).
- Serve API request schema (Unit 04 owns the wire-level `devices`/`profile`
  JSON fields and their validation; Unit 01 only owns what `RecordPlan` does
  once `resolve_plan()` already has resolved values).
- Verification logic beyond what's needed to keep existing single-device checks
  passing (Unit 02 owns the per-device extension).

## Output
- `src/record.rs`: `DeviceCapture`, `ResolvedDevice`, `resolve_devices`,
  `DeviceRun`, `build_stream_for_format` (factored out of the old inline
  `match` on sample format), rewritten `resolve_plan`/`dry_run`/`run`.
- `src/cli.rs`: `RecordArgs::device_track: Vec<String>`,
  `cli::parse_device_track_arg`.
- 6 new unit tests in `record.rs` (`resolve_plan_*`), 3 new in `cli.rs`
  (`device_track_arg_parses_device_name_and_channels` and friends).
- Commit: `feat(record): multi-device concurrent capture`

## Verification
- `cargo test` — 41/41 passing, including the pre-existing
  `fixture::tests::all_fixtures_pass` (proves the refactor didn't disturb the
  audio-routing/24-bit-roundtrip/MIDI-anchor pure functions `record.rs` shares
  with `fixture.rs`).
- `cargo clippy --all-targets --all-features` and `cargo build`/`cargo test`
  under `RUSTFLAGS="-D warnings"` — both clean, matching the repo's actual CI
  job (`.github/workflows/ci.yml`) exactly.
- Manual CLI runs (this sandbox, no real audio hardware): `--device` +
  `--device-track` together → clap rejects with exit 2; an unresolvable
  `--device-track` device → exit 3 (`device_unavailable`) with the right JSON
  shape — confirming the new resolution path threads through the existing
  exit-code taxonomy correctly even without hardware to fully exercise it.
- **Not verified**: real concurrent capture from two actual/virtual devices —
  see SPEC.md §6.
