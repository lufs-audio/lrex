# AGENTS.md — driving lufs-recorder from an agent

lufs-recorder is designed to be operated by an agent as readily as by a human at the keyboard.
This file is the machine-facing contract.

## Discovery

```sh
lufs-recorder --help
lufs-recorder <command> --help
lufs-recorder devices --json      # inventory of audio devices/channels + MIDI ports
```

## The rules

- **`--json` is available on every command.** Prefer it. `devices` returns an inventory;
  `record` and `verify` return a take manifest / verification report. Do not screen-scrape human
  output.
- **Exit codes are the control-flow signal** (see `CONTRACT.md`):
  - `0` — take captured **and** verified. Safe to consume downstream.
  - `3` — device / MIDI port unavailable. Re-run `devices --json` and pick a valid target.
  - `4` — requested format unsupported. Adjust `--rate` / `--bit-depth` / `--channels`.
  - `5` — capture ran but **failed the contract** (xrun, truncation, unbalanced MIDI). The take
    is untrustworthy; do **not** consume it as-is.
  - `6` — interrupted before a valid take.
  - `70` — command not implemented yet (v0.1 skeleton).
- **`--dry-run` pre-flights** a take: it validates device/channel/format availability and predicts
  the output without capturing. Use it to plan a chain before committing.
- **Output is self-describing.** A take is a timestamped directory containing one WAV per captured
  channel, an optional `capture.mid`, and a `take.json` manifest carrying both the captured
  parameters and the verification result. Hand the directory to the next pipeline stage
  (e.g. a Workchain chain) as-is.

## Trust boundary

`verified: true` in `take.json` (and exit `0`) is the *only* signal that a take is correct. A
zero-length recording, a wrong channel count, or a single xrun fails the contract by design — a
recording that "ran" is not a recording that is *right*.

## Design reference

Full spec and rationale: `danialrami/agent-knowledge` → `docs/product/lufs-recorder/`.
