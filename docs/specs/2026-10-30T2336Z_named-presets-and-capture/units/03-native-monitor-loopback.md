# Unit 03 — native-monitor-loopback

## Objective
Add native first-class support for `device = "monitor"` (and `--device monitor`, with aliases `"desktop"` and `"loopback"`), automatically resolving and routing capture to the active audio output stream without requiring manual environment variables (`PULSE_SOURCE`) or modifying system-wide audio input defaults.

## Context
- `src/devices.rs`: `resolve_input_device`, `list_input_devices`, `AudioDeviceInfo`.
- `src/record.rs`: `resolve_plan()`.
- `SPEC.md` §4.2: System Monitor Abstraction.
- Linux PipeWire monitor source (`@DEFAULT_MONITOR@` / `alsa_output.*.monitor`).
- macOS loopback drivers (BlackHole 2ch / 16ch).

## Acceptance criteria
- [ ] Passing `--device monitor`, `--device desktop`, or `--device loopback` (or setting `device = "monitor"` in any config or preset) resolves to the system's active output monitor.
- [ ] On Linux (PipeWire / PulseAudio):
  - Automatically queries the active default audio sink (via `pactl get-default-sink` or PipeWire API).
  - Routes the ALSA/pulse stream directly to that sink's monitor (`<default_sink>.monitor` or `@DEFAULT_MONITOR@`).
  - Does NOT alter the system-wide default input source (physical microphone remains default for other applications like Zoom or Discord).
- [ ] On macOS:
  - Automatically detects installed system loopback devices (e.g. `BlackHole 2ch` or `BlackHole 16ch`).
  - If no loopback device is found, exits with code 3 and outputs a human-readable guidance message: `"No loopback audio device found. Install BlackHole (brew install blackhole-2ch) to record system audio on macOS."`
- [ ] `lrex devices --json` reports a virtual monitor entry in `audio_input_devices` with `name: "monitor"` and `is_monitor: true` indicating platform loopback capability.
- [ ] Dry-run (`lrex record --device monitor --dry-run --json`) succeeds and displays the resolved format and channels.
- [ ] Standard physical devices and custom device substring matching continue to work without regression.

## Interface contract
```rust
// src/devices.rs
pub const MONITOR_DEVICE_ALIASES: &[&str] = &["monitor", "desktop", "loopback"];

pub fn is_monitor_query(query: &str) -> bool;
pub fn resolve_monitor_device() -> std::result::Result<cpal::Device, String>;

// AudioDeviceInfo gains:
// #[serde(default)]
// pub is_monitor: bool,
```
`resolve_input_device(query: &str) -> std::result::Result<cpal::Device, String>` handles `is_monitor_query(query)` prior to searching physical device lists.

## Boundaries — do NOT touch
- Multi-device capture orchestration (`--device-track`)
- MIDI capture or SMF encoding (`src/midi.rs`)
- Contract verification rules in `src/verify.rs`
- Terminal UI layout (`src/tui/`)

## Output
- Modified: `src/devices.rs`, `src/record.rs`
- Unit tests added in `src/devices.rs` covering monitor alias matching and fallback behavior.
- Commit message: `feat(devices): add native monitor/loopback device resolution`

## Verification
```bash
cargo test devices::tests
cargo run --bin lrex -- devices --json
cargo run --bin lrex -- record --device monitor --dry-run --json
```
