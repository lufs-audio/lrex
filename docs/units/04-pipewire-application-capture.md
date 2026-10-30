# Unit 04 — pipewire-application-capture

## Objective
Add an `--app <name>` option to `lrex record` on Linux/PipeWire, isolating and capturing audio exclusively from a specific running application (e.g. `--app "Brave"` or `--app "spotify"`), preventing system notification sounds and unrelated audio from leaking into the recording.

## Context
- `src/cli.rs`: `RecordArgs`.
- `src/devices.rs`: stream routing and node inspection.
- `src/record.rs`: stream lifecycle and capture setup.
- `src/manifest.rs`: take manifest metadata.
- `SPEC.md` §4.3: PipeWire Application Stream Isolation.
- PipeWire tools: `pw-dump`, `pw-link`, PipeWire node properties (`application.name`, `application.process.binary`).

## Acceptance criteria
- [ ] `--app <NAME>` added as an optional argument to `RecordArgs` in `src/cli.rs`.
- [ ] On Linux (PipeWire):
  - When `--app <name>` is provided, `lrex` inspects active PipeWire audio playback streams for matching `application.name` or `application.process.binary` (case-insensitive substring match).
  - If a matching application stream is active, `lrex` routes capture directly to the application's output ports or links the capture stream to that node.
  - If no matching active audio stream exists for that application, `lrex` exits with code 3 (unavailable device/source) and provides a clear error message:
    `"No active audio stream found for application '<name>'. Ensure the application is playing audio and retry."`
  - Audio from other applications (system notification sounds, browser tabs from other processes, chat pings) does NOT appear in the captured audio track.
- [ ] On non-Linux platforms (macOS / Windows) or Linux hosts without PipeWire running:
  - If `--app <name>` is passed, `lrex` exits cleanly with code 4 (unsupported format/feature) and an informative message: `"--app is currently supported on Linux systems running PipeWire."`
- [ ] The generated `take.json` manifest records the captured application name in `captured.devices[].app` or track metadata.
- [ ] Mutually exclusive or well-defined priority: `--app` operates on single-device capture; it cannot be combined with `--device-track`.

## Interface contract
```rust
// src/manifest.rs
// CapturedDeviceInfo / DeviceTrack gains:
pub app: Option<String>,

// src/cli.rs RecordArgs gains:
// #[arg(long, conflicts_with = "device_track", help = "Isolate audio from a specific application (Linux/PipeWire)")]
// pub app: Option<String>,

// src/devices.rs (or src/pipewire.rs)
#[cfg(target_os = "linux")]
pub fn resolve_app_playback_node(app_name: &str) -> Result<u32, String>;
```

## Boundaries — do NOT touch
- MIDI capture engine (`src/midi.rs`)
- Multi-device sample rate alignment logic
- Contract verification calculations in `src/verify.rs` (xruns, duration math)
- TUI monitor event loop (`src/tui/`)

## Output
- Modified: `src/cli.rs`, `src/devices.rs`, `src/record.rs`, `src/manifest.rs`
- Optional new module: `src/pipewire.rs` (isolated PipeWire IPC logic)
- Unit tests verifying CLI parsing, manifest serialization, and mock stream matching.
- Commit message: `feat(pipewire): add --app flag for isolated application audio capture`

## Verification
```bash
cargo test
cargo run --bin lrex -- record --app "NonExistentApp" --dry-run
# With an active audio player running:
cargo run --bin lrex -- record --app "Brave" --duration 5 --name "app-test"
```
