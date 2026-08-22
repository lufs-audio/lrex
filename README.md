# lufs-recorder (`lrex`)

> A universal, agent-first **capture** primitive. Point it at any audio interface, record an
> arbitrary set of that device's channels alongside MIDI, and get back a take that is *proven*
> to be what it claims — right channel count, right sample rate, no dropped frames, MIDI
> time-aligned. One rock-solid CLI. No DAW, no GUI, no hardcoded rig.

lufs-recorder generalizes the Max/MSP patch [`midi-audio-recorder`](https://github.com/danialrami/midi-audio-recorder)
into a portable command-line tool. It is built in **Rust** and is our first production Rust
project — a deliberate flight test for reliability-critical audio tooling in a systems language.

The project/repo keeps its full descriptive name; the binary also installs as **`lrex`**, a
short, typeable alias for daily use (`lufs-audio/bplate` `docs/units/07-lufs-primitive-cli-naming.md`
flags any CLI invocation name over 6 characters, and explicitly recommends an alias — not a
breaking rename — for an existing daily-driver tool like this one). `lrex` and `lufs-recorder`
are the exact same binary under two names; every example below works with either.

**Status: `v0.5.1` — multi-device recorder with named auto-stop profiles, verified, with a browser
control UI and an optional terminal monitor.** The CLI (`record`, `verify`, `devices`, `selftest`,
`init-config`) plus `serve` — a local JSON API + a finalized browser front end (Setup · Console ·
Scope · Glance) with live monitoring — plus `tui`, a terminal-native monitor of that same API. A
take is captured *and verified against the contract* before it is declared good; a take that
dropped frames fails by design. `record` can now capture from multiple devices at once
(`--device-track`) and auto-stop on a named profile (`--profile`) instead of a fixed `--duration`.

## Why it exists

Recording an idea shouldn't mean opening a DAW or living with a rig hardcoded to channels 1–2 and
9–10 and one named keyboard. lufs-recorder keeps the "just hit record" spontaneity of the original
patch while making the device, channels, and MIDI source fully selectable — and, because it's a
*recorder*, it holds every take to a verification contract. A capture that "ran" but silently
dropped samples is worse than one that failed, because it lies to whoever (or whatever agent)
trusted the take.

## Maxpatch parity (the v0.2 defaults)

The built-in defaults reproduce the original patch's rig, so `record` with no flags behaves like
hitting the big toggle in Max:

| | maxpatch | lufs-recorder default |
|---|---|---|
| Audio | ch 1–2 (mic) + ch 9–10 (piano), 24-bit | two stereo tracks `mic` (1–2) + `piano` (9–10), 24-bit |
| MIDI | Nord Stage 3 | `midi_port = "Nord Stage 3"` |
| Output | `~/Samples/sampleLibrary/midi-audio-recorder_max/` | `~/Samples/sampleLibrary/lufs-recorder/<timestamp>_<label>/` |

Everything is overridable — per machine via the config file, per take via flags.

## Build (macOS / klaxon)

A one-shot provisioner is in the repo root. It ensures the C toolchain (Xcode CLT), installs Rust
if needed, and builds `--release` — **both** binaries (`lufs-recorder` and the `lrex` alias) come
out of the same build, no extra step:

```sh
bash provision-and-build.sh            # provision + build
bash provision-and-build.sh --run-devices   # ...and then list your devices/ports
```

Or by hand, with a recent stable Rust toolchain (see `rust-toolchain.toml`):

```sh
cargo build --release
./target/release/lrex --help              # or ./target/release/lufs-recorder --help
```

> **macOS mic permission:** the first `record` triggers a microphone permission prompt for your
> terminal (System Settings ▸ Privacy & Security ▸ Microphone). Approve it once.

## Usage

```sh
# See your devices/channels + MIDI ports (find the exact names).
lrex devices --json

# Write an editable config (device, midi_port, out_dir) — maxpatch parity.
lrex init-config                   # -> ~/.config/lufs-recorder/config.toml

# Pre-flight a take without recording (resolves device/format, predicts output).
lrex record --dry-run --json

# Record the parity rig (mic 1-2 + piano 9-10 + Nord Stage 3); Ctrl-C to stop.
lrex record --name idea

# Override per take: any device, any channels, fixed length, MIDI off.
lrex record --device "Scarlett 18i20" --channels 1-2,9-10 \
  --bit-depth 24 --duration 30 --midi off

# Arbitrary NAMED tracks from the CLI (repeatable). Everything after '=' goes
# into ONE file; capture several MIDI ports at once with a comma list or "all".
lrex record --track mic=1,2 --track piano=9-10 --track room=3-6 \
  --midi "Nord,Syntakt"

# Multi-device: capture two (or more) devices into ONE take. --device-track is
# the sole multi-device surface — DEVICE:NAME=CHANNELS, repeatable. All devices
# in a take must share one sample rate (no cross-device resampling).
lrex record --device-track "BlackHole 2ch:mic=1,2" \
             --device-track "BlackHole 16ch:call=1,2"

# Named auto-stop profiles (configured in config.toml under [profiles.<name>]):
# auto-stops after auto_stop_minutes + a symmetric buffer, instead of --duration.
lrex record --profile therapy

# Re-verify an existing take against the contract.
lrex verify ~/Samples/sampleLibrary/lufs-recorder/2026-07-18_1530_idea --json

# Prove the build itself is correct — no audio hardware needed (audio de-interleave
# + WAV round-trip, MIDI SMF export, A/V anchor math).
lrex selftest --json

# Serve the control UI + JSON API (browser control surface over the engine).
lrex serve --port 8777              # then open http://127.0.0.1:8777/
lrex serve --host 0.0.0.0           # expose on your LAN (other devices)
lrex serve --frontend ./frontend    # live-edit the UI without rebuilding

# Watch a running `serve` from the terminal: live meters, timecode, connection
# state. Read-only monitor -- start/stop stays with `record` or the browser UI.
# Same three standard themes as bplate's wizard (lufs / catppuccin / mono).
lrex tui --port 8777 --theme catppuccin
```

### HTTP API (for a real frontend)

`serve` exposes a small JSON API on `127.0.0.1` and, at `/`, the finalized browser control UI
(built on the LUFS brand system: Setup · Console · Scope · Glance, live meters + a real
oscilloscope over SSE, per-track/mix scopes, an 88-key MIDI roll, the verification checklist, and
`selftest`). The page is embedded in the binary (`--frontend <dir>` overrides it for live UI work).
The stable endpoint contract:

| Method + path | Purpose |
|---|---|
| `GET /api/devices` | audio input devices + MIDI ports |
| `GET /api/config` | resolved config + `out_dir` |
| `GET /api/selftest` | run the in-process fixtures |
| `GET /api/takes` · `GET /api/takes/<id>` | list takes / one take manifest |
| `GET /api/takes/<id>/notes` | parsed MIDI notes `[{channel,key,vel,start_s,dur_s}]` (piano-roll) |
| `GET /api/takes/<id>/waveform?track=<file>&buckets=N` | peak envelope for a fast waveform |
| `GET /api/takes/<id>/file/<name>` | raw WAV/MIDI bytes (Web Audio decode / download) |
| `POST /api/verify` | `{id}` or `{dir}` → manifest + verification |
| `POST /api/record/start` | `{device?, tracks?\|channels?, midi?, rate?, bit_depth?, name?, devices?, profile?, duration?}` — `devices: [{device, tracks:[{name,channels}]}]` for multi-device (additive alongside `device`); `profile: "<name>"` for a named auto-stop instead of `duration`. Giving both `device`+`devices`, or both `profile`+`duration`, is a 400. |
| `GET /api/record/status` | `{recording, name?, elapsed_s?, frames?, xruns?, levels?}` — `levels[]` is per-track `{name,peak_dbfs,rms_dbfs,wave[]}`, live while recording (monitoring) |
| `GET /api/record/stream` | Server-Sent Events: pushes the live snapshot ~12×/s — meters, `wave[]` (signed `[-1,1]` scope samples, ~2400/s, per track) for a real live oscilloscope, `notes[]` (`{key,vel}` note-ons this frame) + `active[]` (held keys) for a live keyboard, and `midi_events` |
| `POST /api/record/stop` | stop → `{stopped, id, take: manifest}` |

The browser is a *control surface*, not the capture engine — multichannel + MIDI capture stay in
the binary. Recording over HTTP currently drives the `record` subprocess (SIGINT to finalize); the
API is stable regardless of that detail.

`lrex tui` is a second, terminal-native surface reading this same `GET /api/record/stream` feed —
also a monitor, not a capture path: it cannot start or stop a take, only watch one already running
via the CLI or the browser UI. It ships with three runtime-selectable themes
(`--theme lufs|catppuccin|mono`) shared with `lufs-audio/bplate`'s `wizard`, the first
implementation of the cross-tool TUI style guide banked at `lufs-audio/bplate`
`references/studies/tui-style-directions/` — so every LUFS terminal tool reads as one family
rather than a per-tool palette.

`--channels` groups by token: a range `1-2` is one stereo track, a bare `1` is its own mono track,
so `1-2,9-10` reproduces the two stereo pairs. `--track NAME=CHANNELS` instead names a track and
puts *all* its channels (commas and ranges) in one file. `--device-track DEVICE:NAME=CHANNELS`
does the same but scoped to one device of several, for multi-device capture. `--midi` takes a port
name, a comma-list of substrings (`"Nord,Syntakt"`), `all`, or `off`. `--json` is available on
every command; exit codes are meaningful (see [CONTRACT.md](CONTRACT.md)) so a supervising agent
can branch on the result.

## Configuration

Resolution order: `--config <FILE>` → `$LUFS_RECORDER_CONFIG` →
`~/.config/lufs-recorder/config.toml` → built-in maxpatch defaults, then an optional project-local
`.lufs-recorder.toml` (in the current directory) is layered on top — only the fields it sets
override, everything else falls through. See [`lufs-recorder.example.toml`](lufs-recorder.example.toml)
for the full annotated file, including the `[profiles.<name>]` table for named auto-stop profiles.
CLI flags override the config; the config overrides the defaults.

## The take

```
<timestamp>_<label>/
  mic.wav               # one WAV per track (a 2-channel track is a stereo file)
  piano.wav
  capture.mid           # present iff MIDI was armed and events occurred
  take.json             # the manifest + verification result
```

`verified: true` in `take.json` (and exit `0`) is the *only* signal that a take is correct.

## Verification (what "good" means)

Every take is measured against what was requested and checked before it's declared good
(see [CONTRACT.md](CONTRACT.md)): files exist and decode, channel count / sample rate / bit depth
match, duration is sane, **`xruns == 0`** (the heart of it), MIDI note-ons balance note-offs, and
the audio isn't digital silence. For a multi-device take, these same checks also run *per device*
(`device_no_xruns`, `device_rate_matches`, etc.) so a failure names which device is at fault, not
just the take as an undifferentiated whole. Dropped frames are detected from the callback capture
timestamps and from ring-buffer overruns. A/V offset is *reported* and gated to a sane bound; the
sub-frame residual will become fully calibrated once the loopback fixture runs on real hardware.

## Design & rationale

The full design record — prior art, the 2026 research landscape, the Rust-vs-C++-vs-Python stack
decision, the verification contract, the flight-test criteria, and the complete tool spec — lives
in the shared knowledge base:

**`lufs-audio/kb` → `docs/product/lufs-recorder/`**

The multi-device + named-profiles feature (v0.5) has its own spec, written per the `speccing`
skill in `lufs-audio/bplate`'s `docs/units` style:

**`docs/specs/multi-device-and-voice-call-profiles/`** (in this repo)

The `tui` command (v0.5.1) follows the cross-tool TUI style guide, banked (per Daniel's decision)
in the `bplate` repo rather than duplicated here since it governs every LUFS terminal tool, not
just this one:

**`lufs-audio/bplate` → `references/studies/tui-style-directions/`**

## Roadmap

See [CHANGELOG.md](CHANGELOG.md) for the full version-by-version history. Currently:

- **v0.1** — honest skeleton: command surface + failing sentinels. *(done)*
- **v0.2** — single-device audio + MIDI, maxpatch parity, config file, inline verification. *(done)*
- **v0.3** — A/V-offset gating; `serve` local control UI + JSON API; take-visualization endpoints. *(done)*
- **v0.4** — live monitoring (per-track peak/RMS + waveform over `status`/SSE); finalized brand-native
  browser front end (Setup · Console · Scope · Glance). *(done)*
- **v0.5** — multi-device concurrent capture (`--device-track`, one sample rate per take, per-device
  verification); named auto-stop profiles + project-scoped config override; the `lrex` short CLI
  alias. *(done — Linux/ALSA concurrent-capture & fault-attribution verified; macOS separate-clock-domain pass open; see `docs/verification-report-v0.5.md` and `TESTING.md`)*
- **v0.5.1** — `lrex tui`: an optional terminal monitor (live meters, timecode, connection state,
  a staleness signal) for a running `serve`, with three runtime-selectable themes (LUFS /
  Catppuccin Mocha / Mono) — the first tool built against the cross-tool TUI style guide banked in
  `lufs-audio/bplate` `references/studies/tui-style-directions/`. *(done — this release.
  Recording-state rendering is schema-verified in a sandbox with no audio hardware, not yet
  watched against a real take — see `TESTING.md` Test 7.)*
- **Later** — NDJSON progress polish; FLAC output; optional live spectrograph bands.
- **v1.0** — hardening, macOS + Linux static binaries, CI running the contract end-to-end.

## License

GPL-3.0-or-later, matching the original maxpatch. See [LICENSE](LICENSE).
