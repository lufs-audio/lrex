# Changelog

All notable changes to this project are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions match `Cargo.toml`.

Entries before this file existed (everything through 0.4.3) are reconstructed from the real
commit and PR history (`git log` / GitHub PRs #2-#17), not invented — dates and PR numbers are
exact. Entries from 0.5.0 onward are written as each change lands.

## [0.5.0] - 2026-08-21

### Added
- Multi-device concurrent capture: `record` can now capture from N devices in one take via
  `--device-track DEVICE:NAME=CHANNELS` (repeatable), each device getting its own ring-buffer +
  writer-thread pipeline sharing one session clock. Every device in a take must resolve to the
  same sample rate (no cross-device resampling; fails closed otherwise). Legacy `--device`/
  `--track`/`--channels` are unchanged. (#19; spec: `docs/specs/multi-device-and-voice-call-profiles/`)
- Named auto-stop profiles: `Config` gains a `profiles` table (`auto_stop_minutes`/
  `buffer_minutes`); `record --profile <name>` sets the capture duration from config instead of
  an explicit `--duration`. Buffer is symmetric: `(auto_stop_minutes + 2*buffer_minutes)*60`
  seconds from actual recording start. (#19)
- Project-scoped config override: an optional `.lufs-recorder.toml` in the current working
  directory layers over the resolved global/explicit config — only the fields it sets change.
  (#19)
- Per-device verification: `verify` now emits `device_no_xruns`/`device_rate_matches`/
  `device_bit_depth_matches`/`device_audio_decodes`/`device_files_exist_nonempty` checks alongside
  the unchanged whole-take aggregates, so a multi-device take's failure names which device is at
  fault. (#19)
- `POST /api/record/start` gains `devices: [{device, tracks:[{name,channels}]}]` (structured,
  additive alongside `device`), `profile`, and `duration` (the API never had `duration` before).
  `device`+`devices` or `profile`+`duration` together, or an unknown profile name, are all
  synchronous 400s — checked before anything spawns. (#19)
- `lrex` — a short (4-character), lowercase, bplate-compliant CLI alias, built alongside the
  existing `lufs-recorder` binary from the same source (`src/lib.rs` + `src/bin/{lufs-recorder,lrex}.rs`).
  Both names run byte-identical behavior; each reports its own name in `--help`/`--version`/error
  output (see `lufs_recorder::invoked_name()`). Per `lufs-audio/bplate` `docs/units/07-lufs-primitive-cli-naming.md`,
  which flags `lufs-recorder` (13 chars) as non-compliant with the 3-6-char convention and
  explicitly recommends an alias — not a breaking rename — for a daily-driver tool with existing
  muscle memory. The package/crate identity, repo name, config path, env var, and default output
  directory are all unchanged.
- Vendored the `speccing` and `land-plane` agent skills into `.agents/skills/` (from
  `danialrami/dotfiles`), matching `lufs-audio/bplate`'s own `.agents/skills/` convention, so an
  agent working in this repo can find the authoring/landing standards without leaving it.
- `take.json` manifest schema bumped to **v3**: `requested`/`captured.device` (singular) became
  `devices` (plural); `RequestedTrack`/`TrackInfo` each gained a `device` field; `captured` gained
  `xruns_by_device`.

### Changed
- `Cargo.toml`'s `repository` field corrected to `lufs-audio/lufs-recorder` (was still pointing at
  the pre-org-transfer `danialrami/lufs-recorder`).
- README's design-record pointer corrected to `lufs-audio/kb` (was the pre-rename
  `danialrami/agent-knowledge` path).
- `record --dry-run --json`'s output is now nested per device (`devices: [...]`) rather than flat
  top-level fields, to represent N devices instead of assuming one.

### Known gaps (flagged, not silently assumed away)
- No real multi-device hardware/virtual-device recording has been exercised through the new
  concurrent-capture code — verification logic is thoroughly unit-tested against fabricated
  multi-device manifests, but `cpal` has no null-device test backend. Top priority for the next
  real-hardware pass (`docs/specs/multi-device-and-voice-call-profiles/SPEC.md` §6).
- No live HTTP smoke test of `serve`'s new `devices`/`profile`/`duration` fields against a running
  instance.

## [0.4.3] - 2026-07-18 (#16, #17)
### Changed
- `levels[].wave` in the live status/SSE stream is now signed `[-1,1]` samples (track channel 0),
  stride-decimated to ~2400 pts/s — a real oscilloscope trace, ~24x denser than the old envelope.
- `serve` gains `--host` (default `127.0.0.1`; `0.0.0.0` for LAN exposure).
### Fixed
- Frontend: centered bipolar oscilloscope rendering from the signed `wave[]` field; 88-key roll
  lights from both `notes[]` (attacks) and `active[]` (held keys) per frame — was `else if`, which
  dropped held keys on attack frames.

## [0.4.2] - 2026-07-18 (#13, #14, #15)
### Added
- Live MIDI in the record stream: a `LiveMidi` accumulator in the input callback; the record
  progress frame carries `notes`/`active`/`midi_events` when MIDI is armed; `serve`'s `/status`
  and SSE `/stream` pass the whole snapshot through.
- Frontend: hidden Shift+D display-tuning panel (oscilloscope/spectrograph/dB-meter ranges,
  persisted to localStorage) and a "time" knob (1-8x) widening the oscilloscope/spectrograph
  history window.

## [0.4.1] - 2026-07-18 (#10, #11, #12)
### Added
- `GET /api/record/stream` — Server-Sent Events pushing the live snapshot ~12x/s (the real live
  scope); `record` emits a per-track decimated peak envelope in NDJSON progress.
- Finalized frontend (Amacher): real API + SSE wiring, Setup as landing page, Console/Scope/Glance,
  mobile per-track scopes, LUFS record-dot favicon, brand fonts.
### Changed
- README brought current to v0.4 status (serve page is the finalized control UI).

## [0.4.0] - 2026-07-18 (#9)
### Added
- Live monitoring: per-track peak/RMS meter emitted in `record`'s NDJSON progress; `serve` relays
  it via `GET /api/record/status` (`frames`, `xruns`, `levels[]`) for real live monitoring while
  recording. Audio callback (real-time path) untouched.

## [0.3.1] - 2026-07-18 (#8)
### Added
- Take-visualization data endpoints for a real frontend: `GET /api/takes/<id>/notes` (parsed MIDI
  notes, metrical + SMPTE timing), `GET /api/takes/<id>/waveform?track=&buckets=N` (peak envelope),
  `GET /api/takes/<id>/file/<name>` (raw WAV/MIDI bytes, path-traversal guarded).

## [0.3.0] - 2026-07-18 (#7)
### Added
- `serve` command (`src/server.rs`): hand-rolled HTTP on `std::net` (no web framework). Endpoints
  for devices/config/selftest/takes/verify/record start-status-stop. Ships an embedded throwaway
  control UI (`--frontend <dir>` overrides it for live iteration). Recording over HTTP shells the
  `record` subprocess and stops it with SIGINT so a take finalizes + verifies exactly like the CLI.
- A/V-offset gating: on a live take with MIDI armed, `verify` now gates that the applied
  MIDI→audio shift is sane (0-500ms); the precise anchor math was already gated tightly by
  `selftest`.

## [0.2.4] - 2026-07-18 (#6)
### Added
- `--track NAME=CHANNELS` (repeatable): name ad-hoc tracks from the CLI; conflicts with
  `--channels`. `--midi` accepts a comma-list of substrings, `all`, or `off`.
- `selftest` command + `src/fixture.rs`: the dependency-free in-process "loopback" — injects a
  known signal into the real pipeline (audio routing + 24-bit round-trip, MIDI balance + hanging-
  note closure + valid SMF + anchor timing, A/V anchor math) with no audio hardware required. Runs
  in CI and on-machine.

## [0.2.3] - 2026-07-18 (#5)
### Changed
- Reverted SMF timing from SMPTE absolute back to metrical (480 TPQN @ 120 BPM) — not all DAWs
  import SMPTE-timed SMF (Logic wouldn't load `capture.mid`), and tempo-relative rescaling is the
  expected/desired behavior for a recorded region.
### Kept from 0.2.2 (unrelated to the revert)
- MIDI→audio anchor + input-latency compensation; hanging-note note-off synthesis at capture end.

## [0.2.2] - 2026-07-18 (#4)
### Fixed
- Two stacked A/V bugs found on real hardware (Logic @ 90 BPM: audio 1.752s vs. MIDI 2.450s):
  MIDI is now anchored to audio sample 0 (`audio_t0 − input_latency`), and notes still held at
  capture end get synthesized note-offs (DAW punch-out behavior) so a held key at Ctrl-C no longer
  false-fails `midi_notes_balanced`.
### Changed
- Manifest schema v2: added `captured.midi_anchor_ns`, `midi.synthesized_note_offs`;
  `av_offset_ms` now reports the applied MIDI→audio shift.

## [0.2.1] - 2026-07-18 (#3)
### Changed
- Config path standardized to `~/.config/lufs-recorder/config.toml` on every platform (was macOS
  `~/Library/Application Support`, which has a space that breaks shell paths).
- `record --dry-run` prints the config file actually loaded (or "(built-in defaults)").

## [0.2.0] - 2026-07-18 (#2)
### Added
- Single-device multichannel audio + MIDI capture at parity with the `midi-audio-recorder`
  maxpatch. Config file (maxpatch-parity defaults), `take.json` manifest + `verify.rs` contract,
  `devices`/`record`/`verify`/`init-config` commands, `provision-and-build.sh` for macOS/klaxon.

## [0.1.0] - 2026-07-07
### Added
- Honest skeleton: the clap command surface (`devices`/`record`/`verify`) parses but every command
  fails with the not-implemented sentinel (exit `70`) per the verifiable-correctness doctrine.
  `AGENTS.md` (agent-first JSON/exit contract), `CONTRACT.md` (verification contract + exit-code
  taxonomy), GPL-3.0 license, self-hosted-fleet CI (`ci.yml`).
