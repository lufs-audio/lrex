# AGENTS.md — driving lufs-recorder from an agent

lufs-recorder is designed to be operated by an agent as readily as by a human at the keyboard.
This file is the machine-facing contract. The binary installs under two identical names —
`lufs-recorder` and the short alias `lrex` (see README's "Build" section) — pick either; every
example below works with both.

## Discovery

```sh
lrex --help
lrex <command> --help
lrex devices --json      # inventory of audio devices/channels + MIDI ports
```

## The rules

- **`--json` is available on every command.** Prefer it. `devices` returns an inventory;
  `record` and `verify` return a take manifest / verification report. Do not screen-scrape human
  output.
- **Exit codes are the control-flow signal** (see `CONTRACT.md`):
  - `0` — take captured **and** verified. Safe to consume downstream.
  - `3` — device / MIDI port unavailable. Re-run `devices --json` and pick a valid target.
  - `4` — requested format unsupported. Adjust `--rate` / `--bit-depth` / `--channels`, or a
    multi-device `--device-track` set that doesn't share one sample rate.
  - `5` — capture ran but **failed the contract** (xrun, truncation, unbalanced MIDI). The take
    is untrustworthy; do **not** consume it as-is. For a multi-device take, check
    `verification.checks[]` for `device_*`-prefixed entries to see which device is at fault.
  - `6` — interrupted before a valid take.
  - `70` — command not implemented yet (v0.1 skeleton).
- **`--dry-run` pre-flights** a take: it validates device/channel/format availability and predicts
  the output without capturing. Use it to plan a chain before committing.
- **Output is self-describing.** A take is a timestamped directory containing one WAV per captured
  channel, an optional `capture.mid`, and a `take.json` manifest carrying both the captured
  parameters and the verification result (schema v3: `requested`/`captured.devices` are arrays;
  each track carries the `device` it came from). Hand the directory to the next pipeline stage
  (e.g. a Workchain chain) as-is.
- **Multi-device and profiles are additive.** `--device-track DEVICE:NAME=CHANNELS` (repeatable)
  is the sole multi-device surface — mutually exclusive with `--device`/`--track`/`--channels`.
  `--profile <name>` sets the auto-stop duration from config — mutually exclusive with
  `--duration`. Neither is required; a plain `record` behaves exactly as it always has.

## Trust boundary

`verified: true` in `take.json` (and exit `0`) is the *only* signal that a take is correct. A
zero-length recording, a wrong channel count, or a single xrun fails the contract by design — a
recording that "ran" is not a recording that is *right*.

## Interactive TUI (a human surface, not an agent one)

`lrex tui` / `lufs-recorder tui` (v0.5.1) is a terminal monitor for a **human** watching a take in
progress — live meters, timecode, connection state. It is explicitly outside the agent-facing
contract above:

- It cannot start or stop a take. It only reads the same `GET /api/record/stream` SSE feed
  `serve` already exposes — no new endpoints, no new capture path.
- An agent should keep using `--json` commands or the HTTP API directly; never screen-scrape the
  TUI's rendered output, which is unstable by nature (themed, laid out for a human eye) unlike the
  `--json` schema, which is the actual stable contract.
- `--theme lufs|catppuccin|mono` only affects this human-facing rendering; it has no bearing on
  anything an agent consumes.

## Design reference

Full spec and rationale: `lufs-audio/kb` → `docs/product/lufs-recorder/`. The multi-device +
named-profiles feature (v0.5) has its own spec in this repo:
`docs/specs/multi-device-and-voice-call-profiles/`. The `tui` command (v0.5.1) follows the
cross-tool TUI style guide banked in `lufs-audio/bplate` → `references/studies/tui-style-directions/`.
