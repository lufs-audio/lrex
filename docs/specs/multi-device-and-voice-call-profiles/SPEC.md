# SPEC.md — Multi-Device Capture + Named Auto-Stop Profiles

*Scoped to this feature (see the placement note in Unit 08 of `lufs-audio/bplate`'s
docs/units for the `docs/specs/<slug>/` convention this follows). Written per the
`speccing` skill (`danialrami/dotfiles agents/.agents/skills/speccing/`), in
bplate's `docs/units` contract style. Status: **implemented** — this document
was updated after the fact to describe what actually shipped, not just what was
planned; see "Where implementation resolved open questions" below.*

## 1. Problem & Motivation
Daniel wants a one-motion "record a voice call" automation: capture mic
(BlackHole 2ch) and incoming call audio (BlackHole 16ch) as one take, auto-stop
on a per-call-type timer, then hand off to transcription. Two confirmed gaps
blocked this (verified via direct code read, 2026-08-21, before any of this
landed): `RecordPlan`/`resolve_plan()` in `src/record.rs` took a single
`device_query: String` — recording was hard-limited to one device at a time, a
gap already named on the README's own Tier-2/post-v1 roadmap; and `config.rs`
had no concept of a named profile or an auto-stop timer, only a one-off
`--duration` CLI flag.

## 2. Goals & Constraints
- Additive, not breaking — every existing single-device, no-profile caller
  keeps working unchanged (proven by `resolve_plan_legacy_single_device_is_unchanged`
  and the manual dry-run smoke tests, not just asserted).
- Backend + config + serve/CLI surface **only** — no TUI. The visual portion
  (meters, timecode, REC dot) is explicitly deferred to the cross-tool TUI
  style guide being scoped with Amacher and does not gate this work.
- Profiles are user-authored config data, never hardcoded product decisions
  (`therapy`/`standup` are illustrative in docs/comments only).
- Extends — never forks — the existing `CONTRACT.md` verification doctrine:
  per-device iteration of existing checks, not a parallel verification system.

## 3. Non-Goals / Boundaries
- The TUI (meters/timecode/REC dot/lazygit-style log) — tracked separately,
  blocked on the Amacher TUI style guide.
- The tscribe handoff (queueing a finished take to a transcription service) —
  that's tscribe's side of the pipeline (see the parallel tscribe generic-tool
  proposal in `danialrami/tscribe-class-carnyx`).
- Deciding the actual `therapy`/`standup` profile names or timings as product
  defaults — that's Daniel's config to write, not this spec's job.
- Cross-device sample-rate resampling — every device in one take must resolve
  to the same rate (see Unit 01); reconciling mismatched native rates is out of
  scope, by design.
- A true multi-device *hardware* capture fixture — see "Honest gap" below.

## 4. Design Approach (as built)
- `RecordPlan` gained `devices: Vec<DeviceCapture>` (`DeviceCapture { device_query,
  tracks }`), replacing the single `device_query`/`tracks` pair. A single-device
  take is simply `devices.len() == 1` — the common case, and behaviorally
  identical to v0.2 through v0.4.
- New CLI flag `--device-track DEVICE:NAME=CHANNELS` (repeatable) is the sole
  multi-device surface — mutually exclusive with `--device`/`--track`/`--channels`
  so there is never ambiguity about which tracks belong to which device.
  Existing flags are untouched for the single-device path.
- `Config` gained a `profiles` map (`name → {auto_stop_minutes, buffer_minutes}`)
  and project-scoped config discovery (`.lufs-recorder.toml`, cwd-relative,
  layered over the global `~/.config` default).
- Verification (`verify.rs`) iterates its existing per-take checks once per
  captured device (`device_no_xruns`, `device_rate_matches`,
  `device_bit_depth_matches`, `device_audio_decodes`,
  `device_files_exist_nonempty`), *alongside* the unchanged whole-take aggregate
  checks — a single-device take degenerates to exactly one device-scoped check
  per dimension, so nothing about existing verification behavior regresses.
- The serve API (`POST /api/record/start`) and CLI both gained `devices`/`profile`
  inputs, additive alongside the existing `device`/`--duration` inputs, with
  synchronous validation (400) when old and new forms collide or a profile name
  is unknown — checked before anything is spawned, not discovered later via an
  async child-process failure.
- `take.json`'s manifest schema bumped to **v3**: `requested.device` /
  `captured.device` (singular) became `devices` (plural); `RequestedTrack` and
  `TrackInfo` each gained a `device` field attributing that track to its source;
  `captured` gained `xruns_by_device`.

## 5. Where implementation resolved open questions
The pre-implementation draft of this spec left a few things deliberately open.
Building it for real resolved them — recorded here rather than silently, per
"don't spec on vibes":

- **Device/track association** — the draft hedged between `Vec<String>` and "a
  small `Vec<DeviceSpec>`". Reading the real `resolve_plan()`/`RecordPlan` made
  the answer concrete: tracks are inherently per-device (a mic device's channels
  are meaningless without knowing which device they're relative to), so the
  shape is `Vec<DeviceCapture>`, each carrying its own `(name, channels)` tracks.
- **The HTTP API shape** — originally approved as flat `devices: [string]`. That
  shape can't express which tracks belong to which device, so it became
  `devices: [{device, tracks: [{name, channels}]}]`, mirroring `--device-track`
  structurally instead of textually. Flagged here rather than shipped silently
  different from what was approved.
- **Cross-device sample rate** — the draft asked "what if devices have different
  native rates?" without answering it. The shipped answer: `resolve_devices()`
  requires every device in a take to resolve to the *same* rate, failing with
  `FormatUnsupported` (exit 4) otherwise — no cross-device resampling. Verified
  by a real CLI run against the sandbox's (device-less) host, confirming the
  error path threads through the existing exit-code taxonomy correctly.
- **Buffer semantics** — confirmed with Daniel (2026-08-21): symmetric,
  `(auto_stop_minutes + 2 × buffer_minutes) × 60`, measured from actual
  recording start. "Start early" is the caller's responsibility; no nominal/
  scheduled-start concept exists anywhere in config or the API.
- **Docs placement** — confirmed with Daniel: `docs/specs/<feature-slug>/` (this
  suite) rather than bplate's flat root `docs/units/`, because lufs-recorder
  already owns `CONTRACT.md`/`AGENTS.md` at root and will accumulate more
  feature specs over time (unlike bplate's one-shot greenfield build).

## 6. Honest gap — no true multi-device hardware fixture
`fixture.rs`'s existing dependency-free self-tests work by writing directly to
`hound::WavWriter` and `midi::write_smf`, bypassing `cpal` entirely — there's no
real or virtual audio device involved. Unit 02's per-device *verification logic*
(attribution, per-device xrun/rate/bit-depth checks) is thoroughly unit-tested
against **fabricated multi-device manifests** (5 tests in `verify.rs`, covering
single-device, one-bad-device-of-two, all-clean, missing-file attribution, and
MIDI-absent). What is **not** tested: an actual end-to-end recording from two
concurrent (real or virtual) devices through `record::run()`'s new
multi-device orchestration. `cpal` has no null/mock device backend, so this
would need either real hardware (BlackHole or similar) or a purpose-built cpal
test harness — neither exists yet. This is the same category of limitation
already on record for `lufs-recorder`'s TESTING.md-style hardware gaps and
`photo-process-registry`'s pointer-grab limitation: real, named, not silently
assumed away. **This is the top priority for Daniel's own hardware
verification pass**, ahead of trusting multi-device capture on a real voice call.

## 7. Units of Work
- **Unit 01 — Multi-Device Capture Engine** (`src/record.rs`, `src/cli.rs`)
- **Unit 02 — Per-Device Verification** (`src/verify.rs`, `src/manifest.rs`)
- **Unit 03 — Named Profiles & Project-Scoped Config** (`src/config.rs`)
- **Unit 04 — HTTP/CLI Surface for Devices & Profiles** (`src/server.rs`, `src/cli.rs`)

## 8. Overall Done-Criteria
Mirrors the original proposal's acceptance criteria, all verified (automated
test or direct CLI run) as of this PR:
- [x] `POST /api/record/start` accepts `devices` (structured) alongside
      (not replacing) `device` — giving both is a 400.
- [x] `POST /api/record/start` and CLI `record` both accept `profile`; when
      given, `auto_stop_minutes`/`buffer_minutes` come from config, not the
      caller. An unknown profile is a synchronous 400 (API) or a clean error
      exit (CLI) — never a silent fallback.
- [x] `take.json`'s schema (v3) attributes every track to its source device and
      breaks xrun counts down per device — verified via fabricated-manifest
      tests; NOT yet verified against a real multi-device hardware take (see
      §6).
- [x] Existing single-device, no-profile callers are unaffected — regression
      tests plus a full `cargo test` pass (41/41) plus manual CLI dry-runs.
- [x] A project-local `.lufs-recorder.toml`, when present, overrides the global
      config for that invocation only — verified both by unit test and a real
      CLI run from a temp project directory.

## 9. Sequencing (as actually built)
Implemented sequentially in one session rather than dispatched to independent
agents, specifically because the units share files more than the original
draft's boundaries assumed (`cli.rs` serves both Unit 01 and Unit 04;
`manifest.rs`'s schema bump serves both Unit 01 and Unit 02). Landed in this
order: docs → Unit 03 (config, no dependents yet) → Unit 01 (record+cli+manifest,
consumes Unit 03's `resolve_profile_duration_secs`) → Unit 02 (verify, consumes
Unit 01's manifest shape) → Unit 04 (server, consumes both 01 and 03). Each
step in this sequence compiles and passes its own tests independently — this
was reasoned through carefully (which fields/functions each step reads vs.
writes) rather than verified by checking out every intermediate commit.
