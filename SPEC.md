# SPEC.md — Named Presets, .lrex.toml Resolution, Native Monitor & App Audio Capture

## Status
Proposed / Ready for review.

## 1. Problem Statement & Motivation
`lrex` (`v0.5.1`) is a reliable capture primitive for audio interfaces and MIDI. However, daily workflows frequently switch between distinct recording contexts:
1. **The Studio Hardware Rig**: The Max/MSP parity setup (physical audio interface, mic on channels 1–2, piano on channels 9–10, 24-bit, MIDI from a physical Nord Stage 3 keyboard).
2. **Desktop System Audio**: Capturing video course audio (e.g. Audinate Dante training), browser lectures, or meeting references (2-channel stereo, 48 kHz, MIDI off).
3. **Application-Isolated Audio**: Capturing clean audio specifically from one program (e.g. Brave browser or Spotify) without capturing desktop notifications, Slack chimes, or system alert sounds.

Currently:
- The user must either manually overwrite `~/.config/lufs-recorder/config.toml` or remember long CLI overrides (`--config <path>`, `--track desktop=1,2`, etc.).
- While the repo and binary have transitioned to `lrex` as the canonical daily CLI alias, configuration discovery still hardcodes `.lufs-recorder.toml`, `~/.config/lufs-recorder/config.toml`, and `LUFS_RECORDER_CONFIG`.
- Capturing desktop audio on Linux requires understanding PipeWire monitor sinks and setting environment variables (`PULSE_SOURCE="@DEFAULT_MONITOR@"`) or globally modifying the desktop's default input source.
- There is no mechanism to isolate audio by application.

## 2. Goals
- **G1: First-Class `lrex` Naming**: Discover `.lrex.toml`, `~/.config/lrex/config.toml`, and `LREX_CONFIG` first, while preserving 100% backwards compatibility with all `.lufs-recorder` equivalents.
- **G2: Named Presets (`--preset <name>`)**: Allow switching between preset configurations (e.g. `lrex record --preset maxpatch` or `lrex record --preset dante`) via named files in `~/.config/lrex/presets/<name>.toml` or inline `[presets.<name>]` tables.
- **G3: Native `monitor` / `desktop` Virtual Device**: Support `device = "monitor"` (or `--device monitor`), automatically resolving the platform's active audio output loopback without requiring manual environment variables or switching default inputs.
- **G4: PipeWire Application Audio Capture (`--app <name>`)**: Allow isolating and recording audio from a specific running application on Linux/PipeWire (e.g. `--app "Brave"`).

## 3. Non-Goals
- **NG1: Not bypassing the verification contract**: Takes captured via presets, monitor loopback, or application streams must undergo the exact same verification contract (sample rate stability, channel count, xrun detection, zero dropped frames).
- **NG2: Not breaking existing configs**: Any config file or CLI command written for `v0.5.1` must continue to function identically.
- **NG3: Not building a full GUI patchbay**: Stream routing is handled deterministically via PipeWire / platform APIs, keeping `lrex` a headless, agent-first CLI primitive.

## 4. Architecture & Design Approach

### 4.1 Config & Preset Hierarchy
When resolving configuration, `lrex` evaluates in priority order:
1. Explicit CLI flags (`--device`, `--rate`, `--track`, etc.)
2. `--config <path>` (explicit path) OR `--preset <name>` (named preset)
3. Project override in `cwd`: `.lrex.toml` -> `.lufs-recorder.toml`
4. Environment variable: `LREX_CONFIG` -> `LUFS_RECORDER_CONFIG`
5. User base config: `~/.config/lrex/config.toml` -> `~/.config/lufs-recorder/config.toml`
6. Built-in defaults

Presets live in `~/.config/lrex/presets/<name>.toml` (and fallback `~/.config/lufs-recorder/presets/<name>.toml`) or as tables inside the base config (`[presets.<name>]`).

### 4.2 System Monitor Abstraction
A reserved device keyword `"monitor"` (aliases `"desktop"`, `"loopback"`):
- On **Linux (PipeWire / PulseAudio)**: Resolves the active default sink monitor (`@DEFAULT_MONITOR@`), routing capture directly to the audio being played through speakers/headphones.
- On **macOS**: Resolves an installed system loopback device (e.g. BlackHole) or informs the user with instructions if no loopback driver is installed.

### 4.3 PipeWire Application Stream Isolation
When `--app <name>` is passed on Linux:
- `lrex` inspects PipeWire's active playback streams via `pw-dump` or `libpipewire`/`spa` node queries, matching `application.name` or `application.process.binary`.
- It links the capture sink directly to the output ports of that specific application's stream node, preventing other system audio from mixing in.

## 5. Decomposition into Units
The work is decomposed into four self-contained, reviewable units:

| Unit | Title | Scope |
|---|---|---|
| **01** | `01-lrex-config-resolution` | `.lrex.toml` cwd discovery, `~/.config/lrex/config.toml`, `LREX_CONFIG` env var with backwards-compatible fallbacks |
| **02** | `02-named-config-presets` | `--preset <name>` CLI option, preset file resolution (`~/.config/lrex/presets/`), and `lrex presets list` |
| **03** | `03-native-monitor-loopback` | `device = "monitor"` / `--device monitor` resolution to active output loopback |
| **04** | `04-pipewire-application-capture` | `--app <name>` flag for isolated PipeWire stream capture on Linux |

## 6. Overall Done Criteria
- [ ] All four unit contracts implemented and passing unit/integration tests.
- [ ] `cargo test` passes cleanly with zero warnings.
- [ ] Existing `v0.5.1` test suites (multi-device, profiles, verification) pass with zero regression.
- [ ] Full dry-run and e2e validation for presets, monitor recording, and application isolation.
