# Proposal: Multi-Device Capture + Named Auto-Stop Profiles

**Status: proposal.** Backend/config scope only — the TUI/visual portion of the "voice-call automation" idea (meters, timecode, REC dot) is explicitly deferred to the cross-tool TUI style guide currently being scoped with Amacher; this document does not depend on that work landing first and can be built independently.

## Motivation
Daniel wants a one-motion "record a voice call" automation: capture his mic (BlackHole 2ch) and the incoming call audio (BlackHole 16ch) as one take, auto-stop on a per-call-type timer, then hand off to transcription. Two confirmed gaps block this today (verified via direct code read, 2026-08-21):
- `RecordPlan`/`resolve_plan()` in `src/record.rs` take a single `device_query: String` — recording is hard-limited to one device at a time. Multi-device is already an acknowledged Tier-2/post-v1 roadmap item in the README, just not built.
- `config.rs` has no concept of a named profile or an auto-stop timer — only a one-off `--duration` CLI flag.

## Proposed shape

### Multi-device capture
- `RecordPlan` gains a `device_queries: Vec<String>` (or a small `Vec<DeviceSpec>` if per-device channel selection needs to differ) instead of a single `device_query`.
- Each device's selected channels are captured into the take's `tracks[]` exactly as today — a two-device take just has tracks sourced from two devices instead of one. The existing `take.json` shape (captured{}, tracks[], verification{}) does not need to change, just where `tracks[].channel` audio is read from.
- Verification gains a per-device xrun/rate/bit-depth check (today's checks already assume a single device; extend to iterate).

### Named profiles
- New config concept: a profile is `{name, auto_stop_minutes, buffer_minutes}` (e.g. `therapy: {auto_stop: 60, buffer: 10}`, `standup: {auto_stop: 30, buffer: 10}`). Buffer applies on both ends (start early / allow overrun) per Daniel's spec.
- `record/start` accepts an optional `profile` field; when present, it sets the auto-stop timer (current time + auto_stop_minutes + buffer) instead of requiring an explicit `--duration`.
- Profiles are user-defined in config, not hardcoded — `therapy` and `standup` are examples, not a fixed enum.
- A project-scoped config override (a `.lufs-recorder.toml` in a project directory, layered over the global `~/.config` default) — this generalizes beyond just the voice-call use case and is worth building at the same time since the mechanism is the same either way.

## Acceptance criteria
- [ ] `POST /api/record/start` accepts `devices: [string]` (plural) in addition to (not replacing, for backward compat) the current singular `device`.
- [ ] `POST /api/record/start` accepts `profile: string`; when given, `auto_stop_minutes`/`buffer_minutes` come from config, not the caller.
- [ ] A take recorded from 2 devices produces one `take.json` with tracks correctly attributed to their source device, and passes verification with per-device checks.
- [ ] Existing single-device callers (no `devices`/`profile` fields) continue to work unchanged — this is additive, not breaking.
- [ ] A project-local config file, when present, overrides the global config for that invocation only.

## Boundaries
- Does not include the TUI (meters/timecode/REC dot/lazygit-style log) — tracked separately, blocked on the Amacher TUI style guide.
- Does not include the tscribe handoff (queueing the finished take to a transcription service) — that's tscribe's side of the pipeline (see the parallel `tscribe` generic-tool proposal in `danialrami/tscribe-class-carnyx`).
- Does not decide the exact `therapy`/`standup` profile names or timings as product defaults — those are Daniel's config to write, not hardcoded into the binary.

## Verification
Extend the existing two-phase verification (plausibility + SHA-256 receipt audit) to cover multi-device takes; add a fixture take with 2 synthetic devices to the test suite.