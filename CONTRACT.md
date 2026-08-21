# CONTRACT.md — what "a good take" means

lufs-recorder inherits the LUFS verifiable-correctness doctrine: **"works" means proven correct,
not merely exited 0.** Every take is checked against this contract before it is declared good.
Failing any required check fails the command (non-zero exit) and marks the manifest
`verified: false`. This contract applies identically regardless of which installed name
(`lufs-recorder` or the `lrex` alias) invoked the command.

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

For a multi-device take (v0.5+, `--device-track`), every assertion above that can vary per device
(xruns, sample rate, bit depth, decode, existence) is checked **once per device** in addition to
the whole-take aggregate, so a failure names which device is at fault
(`verification.checks[]` entries prefixed `device_*`). A/V offset stays whole-take: MIDI is one
armed input, anchored against the take's primary (first) device, not per-device.

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
Likewise: multi-device capture as of v0.5 has thorough per-device verification-logic tests against
fabricated manifests, but no real 2-device hardware/virtual-device recording has been exercised
through it yet — see `docs/specs/multi-device-and-voice-call-profiles/SPEC.md` §6. That gap is
named, not hidden.

Full rationale: `lufs-audio/kb` → `docs/product/lufs-recorder/04-verification-and-flight-test.md`.
