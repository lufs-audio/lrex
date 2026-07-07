# lufs-recorder

> A universal, agent-first **capture** primitive. Point it at any audio interface, record an
> arbitrary set of that device's channels alongside MIDI, and get back a take that is *proven*
> to be what it claims — right channel count, right sample rate, no dropped frames, MIDI
> time-aligned. One rock-solid CLI. No DAW, no GUI, no hardcoded rig.

lufs-recorder generalizes the Max/MSP patch [`midi-audio-recorder`](https://github.com/danialrami/midi-audio-recorder)
into a portable command-line tool. It is built in **Rust** and is our first production Rust
project — a deliberate flight test for reliability-critical audio tooling in a systems language.

**Status: `v0.1` — the honest skeleton.** The full command surface is defined and parses, but no
command is implemented yet. Every command deliberately **fails** with a not-implemented sentinel
(exit code `70`) rather than pretending to work. Nothing here reports success it hasn't earned.

## Why it exists

Recording an idea shouldn't mean opening a DAW or living with a rig hardcoded to channels 1–2 and
9–10 and one named keyboard. lufs-recorder keeps the "just hit record" spontaneity of the original
patch while making the device, channels, and MIDI source fully selectable — and, because it's a
*recorder*, it holds every take to a verification contract. A capture that "ran" but silently
dropped samples is worse than one that failed, because it lies to whoever (or whatever agent)
trusted the take.

## Install / build

```sh
# Requires a recent stable Rust toolchain (see rust-toolchain.toml).
cargo build --release
./target/release/lufs-recorder --help
```

## Usage (target surface)

```sh
# List audio devices/channels and MIDI ports (machine-readable).
lufs-recorder devices --json

# Capture channels 1-2 and 9-10 at 48k/24-bit, arm the Nord, run until Ctrl-C.
lufs-recorder record --device "Scarlett 18i20" --channels 1-2,9-10 \
  --rate 48000 --bit-depth 24 --midi "Nord Stage 3"

# Pre-flight a take without recording.
lufs-recorder record --device "Scarlett 18i20" --channels 1,2 --dry-run --json

# Re-verify an existing take against the contract.
lufs-recorder verify ./2026-07-07_1530_idea --json
```

Every command supports `--json`. Exit codes are meaningful (see [CONTRACT.md](CONTRACT.md)) so a
supervising agent can branch on the result.

## Design & rationale

The full design record — prior art, the 2026 research landscape, the Rust-vs-C++-vs-Python stack
decision, the verification contract, the flight-test criteria, and the complete tool spec — lives
in the shared knowledge base:

**`danialrami/agent-knowledge` → `docs/product/lufs-recorder/`**

## Roadmap

- **v0.1** — honest skeleton: command surface + failing sentinels. *(you are here)*
- **v0.2** — single-device audio MVP: enumerate, select device/channels, capture to WAV,
  deterministic contract checks (channel count, rate, `xruns == 0`, decodes).
- **v0.3** — MIDI + shared monotonic clock: SMF export, A/V offset check. Full maxpatch parity.
- **v0.4** — `take.json` manifest, `--dry-run`, NDJSON progress; loopback fixture at certify-time.
- **v1.0** — hardening, macOS + Linux static binaries, CI running the contract.
- **Tier 2 (best-effort, post-v1)** — multi-device simultaneous capture via per-OS backends, with
  documented clock-drift risk. Does *not* gate v1.

## License

GPL-3.0-or-later, matching the original maxpatch. See [LICENSE](LICENSE).
