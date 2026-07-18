# lufs-recorder

> A universal, agent-first **capture** primitive. Point it at any audio interface, record an
> arbitrary set of that device's channels alongside MIDI, and get back a take that is *proven*
> to be what it claims — right channel count, right sample rate, no dropped frames, MIDI
> time-aligned. One rock-solid CLI. No DAW, no GUI, no hardcoded rig.

lufs-recorder generalizes the Max/MSP patch [`midi-audio-recorder`](https://github.com/danialrami/midi-audio-recorder)
into a portable command-line tool. It is built in **Rust** and is our first production Rust
project — a deliberate flight test for reliability-critical audio tooling in a systems language.

**Status: `v0.2` — single-device capture with MIDI, at parity with the maxpatch.** `record`,
`verify`, `devices`, and `init-config` are implemented. A take is captured *and verified against
the contract* before it is declared good; a take that dropped frames fails by design.

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
if needed, and builds `--release`:

```sh
bash provision-and-build.sh            # provision + build
bash provision-and-build.sh --run-devices   # ...and then list your devices/ports
```

Or by hand, with a recent stable Rust toolchain (see `rust-toolchain.toml`):

```sh
cargo build --release
./target/release/lufs-recorder --help
```

> **macOS mic permission:** the first `record` triggers a microphone permission prompt for your
> terminal (System Settings ▸ Privacy & Security ▸ Microphone). Approve it once.

## Usage

```sh
# See your devices/channels + MIDI ports (find the exact names).
lufs-recorder devices --json

# Write an editable config (device, midi_port, out_dir) — maxpatch parity.
lufs-recorder init-config          # -> ~/.config/lufs-recorder/config.toml

# Pre-flight a take without recording (resolves device/format, predicts output).
lufs-recorder record --dry-run --json

# Record the parity rig (mic 1-2 + piano 9-10 + Nord Stage 3); Ctrl-C to stop.
lufs-recorder record --name idea

# Override per take: any device, any channels, fixed length, MIDI off.
lufs-recorder record --device "Scarlett 18i20" --channels 1-2,9-10 \
  --bit-depth 24 --duration 30 --midi off

# Arbitrary NAMED tracks from the CLI (repeatable). Everything after '=' goes
# into ONE file; capture several MIDI ports at once with a comma list or "all".
lufs-recorder record --track mic=1,2 --track piano=9-10 --track room=3-6 \
  --midi "Nord,Syntakt"

# Re-verify an existing take against the contract.
lufs-recorder verify ~/Samples/sampleLibrary/lufs-recorder/2026-07-18_1530_idea --json

# Prove the build itself is correct — no audio hardware needed (audio de-interleave
# + WAV round-trip, MIDI SMF export, A/V anchor math).
lufs-recorder selftest --json

# Serve a local control UI + JSON API (browser control surface over the engine).
lufs-recorder serve --port 8777          # then open http://127.0.0.1:8777/
lufs-recorder serve --frontend ./frontend   # live-edit the UI without rebuilding
```

### HTTP API (for a real frontend)

`serve` exposes a small JSON API on `127.0.0.1`; the bundled page at `/` is a *throwaway*
control surface that exercises it. The stable endpoint contract:

| Method + path | Purpose |
|---|---|
| `GET /api/devices` | audio input devices + MIDI ports |
| `GET /api/config` | resolved config + `out_dir` |
| `GET /api/selftest` | run the in-process fixtures |
| `GET /api/takes` · `GET /api/takes/<id>` | list takes / one take manifest |
| `POST /api/verify` | `{id}` or `{dir}` → manifest + verification |
| `POST /api/record/start` | `{device?, tracks?|channels?, midi?, rate?, bit_depth?, name?}` |
| `GET /api/record/status` | `{recording, name?, elapsed_s?}` |
| `POST /api/record/stop` | stop → `{stopped, id, take: manifest}` |

The browser is a *control surface*, not the capture engine — multichannel + MIDI capture stays in
the binary. Recording over HTTP currently drives the `record` subprocess (SIGINT to finalize); the
API is stable regardless of that detail.

`--channels` groups by token: a range `1-2` is one stereo track, a bare `1` is its own mono track,
so `1-2,9-10` reproduces the two stereo pairs. `--track NAME=CHANNELS` instead names a track and
puts *all* its channels (commas and ranges) in one file. `--midi` takes a port name, a comma-list
of substrings (`"Nord,Syntakt"`), `all`, or `off`. `--json` is available on every command; exit
codes are meaningful (see [CONTRACT.md](CONTRACT.md)) so a supervising agent can branch on the result.

## Configuration

Resolution order: `--config <FILE>` → `$LUFS_RECORDER_CONFIG` →
`~/.config/lufs-recorder/config.toml` → built-in maxpatch defaults. See
[`lufs-recorder.example.toml`](lufs-recorder.example.toml) for the full annotated file. CLI flags
override the config; the config overrides the defaults.

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
the audio isn't digital silence. Dropped frames are detected from the callback capture timestamps
and from ring-buffer overruns. A/V offset is *reported* in v0.2 and will become a gating check once
the loopback fixture calibrates it.

## Design & rationale

The full design record — prior art, the 2026 research landscape, the Rust-vs-C++-vs-Python stack
decision, the verification contract, the flight-test criteria, and the complete tool spec — lives
in the shared knowledge base:

**`danialrami/agent-knowledge` → `docs/product/lufs-recorder/`**

## Roadmap

- **v0.1** — honest skeleton: command surface + failing sentinels. *(done)*
- **v0.2** — single-device audio + MIDI, maxpatch parity, config file, inline verification. *(done)*
- **v0.2.x** — `~/.config` standard path; MIDI→audio anchor + latency comp; hanging-note closure;
  arbitrary named tracks (`--track`), multi-port MIDI; in-process `selftest` fixture.
- **v0.3** — A/V-offset gating (sane-bound on the live take; math gated by `selftest`); `serve`
  local control UI + JSON API (throwaway frontend + stable endpoints). *(you are here)*
- **v0.4** — a real frontend (Amacher) over the `serve` API; NDJSON progress polish; FLAC output.
- **v1.0** — hardening, macOS + Linux static binaries, CI running the contract end-to-end.
- **Tier 2 (best-effort, post-v1)** — multi-device simultaneous capture via per-OS backends, with
  documented clock-drift risk. Does *not* gate v1.

## License

GPL-3.0-or-later, matching the original maxpatch. See [LICENSE](LICENSE).
