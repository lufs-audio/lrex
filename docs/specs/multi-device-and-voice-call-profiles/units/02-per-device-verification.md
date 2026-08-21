# Unit 02 — Per-Device Verification

*Status: implemented (this PR), with an honest scope reduction from the
pre-implementation draft's title ("...& Multi-Device Fixture") — see below.*

## Objective
Extend the existing two-phase verification (`CONTRACT.md`) to iterate its
per-take checks (xrun, rate, bit-depth, decode, existence) once per captured
device instead of assuming a single device, so a multi-device take's failure
names *which* device is at fault.

## Context
- `CONTRACT.md` — the deterministic assertions this must keep honoring (no
  parallel verification philosophy; this extends it)
- SPEC.md (this suite), §4 and §6
- Code: `src/verify.rs` (`PerDevice`, `run`), `src/manifest.rs` (schema v3:
  `Requested.devices`, `Captured.devices`/`xruns_by_device`,
  `RequestedTrack.device`, `TrackInfo.device`)
- Depends on Unit 01's per-track `device` attribution existing in `take.json`

## Acceptance criteria
- [x] Each device represented in a take's `tracks[]` gets its own xrun check
      (`device_no_xruns`), rate check (`device_rate_matches`), and bit-depth
      check (`device_bit_depth_matches`) — plus `device_audio_decodes` and
      `device_files_exist_nonempty`, which the original draft didn't call out
      by name but are the same class of per-device attribution.
- [x] A take with N devices reports `verification.checks[]` entries that
      identify which device failed, if any did — each `device_*` check's
      `detail` names the device explicitly.
- [x] A single-device take's verification output shape is unchanged —
      `no_xruns`, `channel_count_matches`, `sample_rate_matches`,
      `bit_depth_matches`, `files_exist_nonempty`, `audio_decodes` are all
      still emitted exactly as before, with the `device_*` checks purely
      additive (a single-device take gets exactly one `device_*` check per
      dimension, true under the identical condition as the aggregate).
- [x] A clean multi-device take verifies true; an induced xrun on one device
      fails that device's `device_no_xruns` check without failing the other
      device's — proven by `multi_device_xrun_on_one_device_is_individually_attributable`.
- [~] **Scope reduction, flagged rather than silently dropped**: the original
      draft's title/verification section called for "a fixture take with 2
      synthetic devices" in the test suite. What's built instead is 5 tests in
      `verify.rs` against **fabricated multi-device manifests** (constructing
      `Manifest`/`Captured`/`TrackInfo` structs directly with multi-device
      shapes) — this thoroughly proves the *attribution logic* but does not
      exercise a real recording through `record::run()`'s device-orchestration
      code. See SPEC.md §6 for why (`cpal` has no null-device test backend,
      unlike `fixture.rs`'s existing WAV-writer-bypass approach) and what a real
      fixture would need.

## Interface contract
```json
"captured": {
  "devices": ["BlackHole 2ch", "BlackHole 16ch"],
  "xruns": 3,
  "xruns_by_device": [
    {"device": "BlackHole 2ch", "xruns": 0},
    {"device": "BlackHole 16ch", "xruns": 3}
  ]
}
"verification": {
  "verified": false,
  "checks": [
    {"name": "no_xruns", "ok": false, "gating": true, "detail": "3 dropped-frame/overflow event(s)"},
    {"name": "device_no_xruns", "ok": true, "gating": true},
    {"name": "device_no_xruns", "ok": false, "gating": true, "detail": "device 'BlackHole 16ch': 3 dropped-frame/overflow event(s)"}
  ]
}
```
The whole-take aggregate checks (`no_xruns`, etc.) are unchanged in name and
semantics — `device_*` checks are additive, never a replacement.

## Boundaries — do NOT touch
- Device capture / concurrency (Unit 01).
- Config/profile schema (Unit 03).
- The exit-code taxonomy in `CONTRACT.md` (stays 0/2/3/4/5/6/70 — a per-device
  failure still surfaces as exit 5 for the take as a whole).

## Output
- `src/verify.rs`: `PerDevice` (deliberately not `#[derive(Default)]` — see the
  doc comment on why that would silently invert the pass/fail default), the
  per-device breakdown loop, 5 new tests.
- `src/manifest.rs`: schema v3 fields (see Unit 01/02 shared interface above).
- Commit: `feat(verify): per-device checks + manifest schema v3`

## Verification
- `cargo test`: 5 dedicated tests
  (`single_device_clean_take_has_one_device_no_xruns_check_and_passes_it`,
  `multi_device_xrun_on_one_device_is_individually_attributable`,
  `multi_device_all_clean_verifies_true_for_the_xrun_dimension`,
  `per_device_file_checks_attribute_a_missing_track_to_its_device`,
  `midi_absent_is_not_gated`), all passing.
- `cargo clippy`/`fmt` clean (see Unit 01's verification section — same
  whole-crate run covers this file).
- **Not verified**: a real 2-device hardware/virtual-device take passing
  end-to-end through `record::run()` into `verify::run()` — see SPEC.md §6.
  This is the single highest-priority gap for Daniel's own hardware pass.
