# CONTRACT.md — what "a good take" means

lufs-recorder inherits the LUFS verifiable-correctness doctrine: **"works" means proven correct,
not merely exited 0.** Every take is checked against this contract before it is declared good.
Failing any required check fails the command (non-zero exit) and marks the manifest
`verified: false`.

## Exit-code taxonomy (stable)

| Code | Meaning |
|------|---------|
| `0`  | Take captured **and** verified |
| `2`  | Bad usage / arguments (clap) |
| `3`  | Requested audio device or MIDI port unavailable |
| `4`  | Requested format (rate / bit-depth / channels) unsupported by device |
| `5`  | Capture ran but **failed the contract** |
| `6`  | Interrupted before a valid take |
| `70` | Not implemented yet (honest-failure sentinel) |

## Deterministic assertions (every take)

- **Outputs exist and are non-empty** — a WAV per requested track, `capture.mid` iff MIDI armed,
  and `take.json`.
- **Audio decodes** — WAV headers valid; the stream reads end-to-end.
- **Channel count matches** — captured channels equal the requested channel subset.
- **Sample rate & bit depth match** — the device actually ran at the requested format.
- **Duration is sane** — `frames / sample_rate` ≈ requested (or wall-clock) duration, within tolerance.
- **No dropped frames / xruns** — the capture callback's overflow counter is **zero**. This is the
  heart of the contract: a glitch-free take is the product.
- **MIDI event integrity** — when armed and notes were played, event count > 0, the SMF parses, and
  every note-on has a matching note-off (or is explicitly flagged).
- **A/V offset within tolerance** — measured audio-start-to-MIDI-`t0` offset ≤ the declared bound.

## Metamorphic checks (creative capture has no single right answer)

- **Duration preservation** — audio duration ≈ MIDI timeline span (within latency tolerance).
- **Structural determinism** — identical requested config ⇒ identical track count, file naming, and
  manifest shape; take-ids are a deterministic content hash.
- **Loudness sanity** — integrated LUFS / true-peak land in a plausible band (not digital silence,
  not fully clipped).
- **Loopback fixture (`test`/certify time)** — a known signal (e.g. 1 kHz tone or click train) fed
  through a loopback/virtual device is recovered within tolerance.

Cheap relations run on every execution; the expensive loopback fixture runs at test/certify time.

## Honest failure

Unimplemented functionality fails loudly (exit `70`) — the sentinel lives in the code, not just in
documentation. Nothing reports success it has not earned, starting from the very first commit.

Full rationale: `danialrami/agent-knowledge` → `docs/product/lufs-recorder/04-verification-and-flight-test.md`.
