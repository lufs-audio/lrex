# Unit 04 — HTTP/CLI Surface for Devices & Profiles

*Status: implemented (this PR), with one real architectural gap found and
fixed along the way — see "A gap found while implementing" below.*

## Objective
Expose multi-device and profile capabilities through the serve HTTP API
(`POST /api/record/start`) and the CLI, additively alongside the existing
singular `device`/`--duration` inputs, with explicit validation when old and
new forms collide.

## Context
- Serve API contract (`POST /api/record/start` previously accepted
  `name, device?, channels?|tracks?, midi?, rate?, bit_depth?` — notably no
  `duration` field existed in the API at all before this unit)
- SPEC.md (this suite) §5 — the API shape correction (structured `devices`,
  not flat strings)
- Depends on Unit 01 (`RecordPlan` accepting multiple devices via
  `--device-track`) and Unit 03 (profile-name → duration) already existing;
  this unit is integration/wiring, not new capture or config logic

## Acceptance criteria
- [x] `POST /api/record/start` accepts optional `devices: [{device, tracks:
      [{name, channels}]}]`; when present, it's translated into repeated
      `--device-track DEVICE:NAME=CHANNELS` args for the spawned child. Giving
      both `device` and `devices`: 400, checked before anything is spawned.
- [x] `POST /api/record/start` accepts optional `profile: string` (and, newly,
      `duration: number` — see the gap below); giving both `profile` and
      `duration`: 400.
- [x] An unknown profile name returns a clear 400 — see "A gap found" below for
      why this needed a real fix, not just a field addition.
- [x] CLI gains equivalent surface: `--device` stays exactly as before;
      `--profile <name>` is mutually exclusive with `--duration` at the clap
      parser level (`conflicts_with`) — verified by an actual CLI invocation
      with both flags, not just reading the attribute.
- [x] `--json` output already reflects resolved state via the existing NDJSON
      progress relay (unchanged plumbing) — no new fields needed here since
      `serve` already passes the child's whole snapshot through generically.
- [x] Existing callers using only `device`/`--duration` see no change in
      behavior or response shape (the legacy branch of `api_record_start`'s
      arg-building is untouched code, only reached when `devices`/`profile`
      are absent).

## A gap found while implementing
`api_record_start` works by shelling out to the compiled binary and returning
`{"recording": true}` **before** the child proves it actually started — the
existing architecture (unchanged by this unit) discovers a bad `device` name
only asynchronously, when `/api/record/status` next polls and reaps the
self-exited child. That pattern would have made "unknown profile name → 400"
impossible to honor literally, since the parent has no synchronous signal from
the child.

Fix: `state.cfg.profiles` (the already-loaded config) is checked directly in
`api_record_start`, before spawning anything. This gives a *real* synchronous
400 for an unknown profile — not merely a field that gets passed through and
fails later. Device-name validity is unchanged (still async, matching the
pre-existing pattern for `device`) since that would need opening the device
inside the HTTP handler, a larger change out of scope here.

## Interface contract
```
POST /api/record/start
{
  "name": "string",
  "device": "string",                        // existing, singular
  "devices": [                                // NEW, structured — not devices:[string]
    {"device": "string", "tracks": [{"name": "string", "channels": [1,2]}]}
  ],
  "profile": "string",                        // NEW — mutually exclusive with duration
  "duration": 0.0,                            // NEW — the API never had this field before
  "channels": "string", "tracks": [...], "midi": "string", "rate": 0, "bit_depth": "string"
}
```
`device`+`devices` together, or `profile`+`duration` together: 400. An unknown
`profile`: 400 (checked against `state.cfg.profiles` synchronously).

## Boundaries — do NOT touch
- `RecordPlan` internals / capture loop (Unit 01).
- Config schema / profile resolution math (Unit 03) — this unit calls into it,
  doesn't reimplement it.
- Verification (Unit 02).

## Output
- `src/server.rs`: `DeviceTrackReq`/`TrackReq` request structs, validation
  block in `api_record_start`, arg-translation for `devices`/`profile`/
  `duration`, updated module-doc endpoint contract.
- `src/cli.rs`: `--profile` flag (already landed as part of Unit 01/03's shared
  CLI surface — this unit's contribution is the *serve*-side wiring).
- 2 new unit tests (request-shape validation and parsing).
- Commit: `feat(api): expose multi-device + profile inputs on record/start`

## Verification
- `cargo test`: 2 dedicated tests, both passing.
- Manual CLI runs proving the underlying flags this unit wires actually work:
  `--duration`+`--profile` together → clap rejects, exit 2 (same mechanism the
  HTTP layer's synchronous profile/duration check mirrors).
- `cargo clippy`/`fmt` clean.
- **Not verified**: an actual HTTP request against a running `serve` instance
  (would need the sandbox to bind a port and issue real requests end-to-end;
  the request-shape and validation-logic tests cover the parts that don't need
  a live server, but a live-server smoke test is still open for Daniel's own
  pass, same category as SPEC.md §6).
