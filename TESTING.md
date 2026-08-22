# TESTING.md — real-hardware verification for lufs-recorder / lrex v0.5

## Why this exists

v0.5 (multi-device capture, `--device-track`; named auto-stop profiles, `--profile`; the `lrex`
alias) was built and verified end to end in a **sandbox with no audio hardware at all**. Everything
that can be proven without real audio I/O has been: `cargo test` (41/41, including the
dependency-free `fixture::tests::all_fixtures_pass` loopback), `clippy`/`fmt`/`build` under
`RUSTFLAGS="-D warnings"` (matching `.github/workflows/ci.yml` exactly), CLI conflict-validation
and exit-code behavior against fake device names, and a real end-to-end project-config-override
test from a temp directory.

**What has never once run against a real or virtual audio device: the actual multi-device capture
path** (`record::run()`'s new per-device ring-buffer/writer-thread orchestration), **named-profile
auto-stop timing in real wall-clock time**, and **the `serve` HTTP API's new `devices`/`profile`/
`duration` fields against a live running instance**. `cpal` has no null-device test backend, so
none of that is fakeable — it needs a real machine. That's what this runbook closes.

v0.5.1 adds `lrex tui`, a terminal monitor of `serve`'s existing status/stream API. It was built
and verified the same way — see Test 7 for the one real-hardware gap it has (watching a **live**
take; the idle/unreachable/reconnect states are already PTY-verified against a real `serve`
process, not just unit-tested).

Each step below has an exact command, an **expected result**, and a **stop condition** — if you
hit the stop condition, don't try to patch the source yourself; capture the evidence (full console
output, the relevant `take.json`, the exit code) and report it back (see "Reporting back" at the
end) so any real fix lands as a reviewed PR, not an ad-hoc hotfix on a daily-driver tool.

## Priority order

Do these in order. 1-3 are the never-before-tested, highest-value items; 4-6 are regression/sanity
checks that *should* be unaffected by v0.5 but are worth confirming on real hardware since nothing
here ran there before either; 7 is v0.5.1's one real-hardware gap.

1. **Multi-device capture** (Test 3) — the flagship test. Highest priority by far.
2. **Named profile auto-stop timing** (Test 4) — never run in real wall-clock time.
3. **Live `serve` API smoke test** (Test 6) — never hit with a real HTTP request.
4. Baseline single-device regression (Test 2).
5. Project-scoped config override, for real (Test 5).
6. `lrex` alias sanity (Test 1) — lowest risk, but quick, do it first as a sanity gate.
7. **TUI live monitoring** (Test 7) — confirms `lrex tui`'s meters/timecode against a real take;
   everything about it was schema-verified in a sandbox, never watched against real audio.

## Prerequisites

- A macOS machine (this tool's primary target — `provision-and-build.sh` calls out macOS/klaxon
  specifically, and BlackHole, the virtual device used in the motivating voice-call use case, is
  macOS-only). Linux works too (ALSA); see the note in Test 3 if you're on Linux and want a
  BlackHole-equivalent virtual loopback.
- Rust toolchain (rustup) — `provision-and-build.sh` installs it if missing.
- **At least two distinct audio input devices the OS can see.** The ideal match for the motivating
  use case is BlackHole 2ch + BlackHole 16ch (Existential Audio, free —
  `brew install blackhole-2ch blackhole-16ch` or https://existential.audio/blackhole/), but Test 3
  does **not** require exactly those — any two real input devices (built-in mic + a USB interface,
  built-in mic + a BlackHole fed by any system audio, two USB interfaces) prove the same code path.
- A real microphone the OS recognizes (built-in is fine) for the baseline test.
- If you already have a real `~/.config/lufs-recorder/config.toml` with your own device/track
  setup: **do not overwrite it**. Test 4 tells you how to add a throwaway profile to it safely.

## Setup

```sh
git clone https://github.com/lufs-audio/lufs-recorder.git
cd lufs-recorder
bash provision-and-build.sh
```

**Expected:** both `target/release/lufs-recorder` and `target/release/lrex` exist and are
executable (the script prints both paths at the end).
**Stop condition:** the build fails. Report the exact compiler error verbatim — a real-hardware
build failure could mean something the sandbox's toolchain genuinely couldn't catch (e.g. a
platform-specific linking issue), which would be valuable to know precisely, not paraphrase.

## Test 1 — `lrex` alias sanity

```sh
./target/release/lrex --version
./target/release/lufs-recorder --version
./target/release/lrex --help | head -3
./target/release/lufs-recorder --help | head -3
```

**Expected:** `lrex --version` prints `lrex 0.5.1`; `lufs-recorder --version` prints
`lufs-recorder 0.5.1`; each `--help`'s `Usage:` line names the binary it was actually invoked as.
**Stop condition:** either binary reports the wrong name for itself. This was verified in the
sandbox already, so a failure here on real hardware would be surprising and worth flagging clearly.

## Test 2 — baseline single-device regression

```sh
./target/release/lrex devices --json
```

**Expected:** valid JSON listing your real input devices (at least the built-in mic) and MIDI
ports (an empty list is fine if nothing's connected).

The **built-in** config default (mic 1-2 + piano 9-10) assumes a 10+ channel interface — on a
laptop with only a 1-2 channel built-in mic, a plain `record --dry-run` will correctly fail with
exit `4` (`FormatUnsupported`). **That's expected, not a bug.** Override to match your real device:

```sh
./target/release/lrex record --dry-run --channels 1-2 --midi off --json
```

**Expected:** dry-run JSON resolving your default input device at some real sample rate.

Then a real short recording:

```sh
./target/release/lrex record --channels 1-2 --midi off --duration 5 --name baseline-test
```

**Expected:** exit `0`, console prints `VERIFIED`, and a take directory appears under
`~/Samples/sampleLibrary/lufs-recorder/<timestamp>_baseline-test/` with a WAV file and a
`take.json` where `verification.verified: true` and `captured.xruns: 0`.
**Stop condition:** exit `!= 0`, `verified: false`, or `xruns > 0` — every prior release already
proved this exact path works, so a failure here is high-signal. Report the full `take.json` and
console output.

## Test 3 — THE FLAGSHIP TEST: real multi-device capture

First, find two real device names:

```sh
./target/release/lrex devices --json
```

Pick two entries from `audio_input_devices[]` (e.g. `"BlackHole 2ch"` and `"BlackHole 16ch"` if
installed, or your built-in mic plus any other interface). Note each device's real channel count
so you request channels it actually has.

Route *some* real audio into both — it doesn't need to be an actual phone call for this first
pass; test tones or `afplay`-ing a file into a device work fine. What matters is that both devices
are actively receiving audio at record time.

```sh
./target/release/lrex record \
  --device-track "DEVICE_A_NAME:trackA=1,2" \
  --device-track "DEVICE_B_NAME:trackB=1,2" \
  --duration 8 --name multidevice-test
```

(Replace `DEVICE_A_NAME`/`DEVICE_B_NAME` with the exact strings from `devices --json`, and adjust
channel numbers to what each device actually exposes.)

**Expected (success path):** exit `0`; the take directory contains **both** `tracka.wav` and
`trackb.wav`; `take.json` shows `captured.devices` with both real device names, `tracks[]` with 2
entries each carrying the correct `device` field, `captured.xruns_by_device` with 2 entries (ideally
both `xruns: 0`), and `verification.checks[]` containing `device_no_xruns` / `device_rate_matches` /
`device_bit_depth_matches` / `device_audio_decodes` / `device_files_exist_nonempty` **twice** (once
per device), all `ok: true`.

**Expected (a real, correct failure mode):** if your two devices don't share a native sample rate
(e.g. built-in mic at 48000 Hz vs. a USB interface defaulting to 44100 Hz), this fails with exit
`4` and a message naming both devices and both rates — **this is the code working as designed**
(no silent cross-device resampling), not a bug. Retry with `--rate` pinned to a rate both devices
support, per the error message.

**Report either way** — full `take.json` on success, or the full error text + exit code on
failure. This is the single most valuable piece of real-world data from this whole runbook: this
exact code path (concurrent per-device ring buffers/writer threads sharing one session clock) has
never executed against real hardware before.

*Linux note:* if you don't have two physical devices, `snd-aloop` (ALSA loopback kernel module)
can create virtual loopback device pairs as a BlackHole-equivalent — `sudo modprobe snd-aloop`.

## Test 4 — named profile auto-stop timing

If you already have a real config, **add** this to it rather than overwriting (do not run
`init-config --force` if `~/.config/lufs-recorder/config.toml` already has your real setup):

```toml
[profiles.quick-test]
auto_stop_minutes = 0.1
buffer_minutes = 0.05
```

(Total expected duration: `(0.1 + 2*0.05) * 60` = 12 seconds — short on purpose so this test
doesn't take an hour.)

```sh
time ./target/release/lrex record --channels 1-2 --midi off --profile quick-test --name profile-test
```

**Expected:** the command runs for **approximately 12 seconds** (allow a couple seconds either way
for process start/stop overhead) and **exits on its own** — you should not need to press Ctrl-C.
Exit `0`, `verified: true`, and `take.json`'s `requested.duration_s` ≈ 12.
**Stop condition:** if it doesn't auto-stop within ~30s, press Ctrl-C and report what happened.
The sandbox's 9 profile-math unit tests only prove the *arithmetic* — they can't prove
`record::run()` actually threads the resulting duration into a live capture loop's stop condition,
since that requires a real device to record from.

## Test 5 — project-scoped config override, for real

```sh
mkdir -p /tmp/lrex-project-test && cd /tmp/lrex-project-test
cat > .lufs-recorder.toml <<'EOF'
device = "REPLACE_WITH_A_REAL_DEVICE_SUBSTRING"
EOF
/full/path/to/target/release/lrex record --dry-run --channels 1-2 --midi off --json
```

**Expected:** the dry-run JSON's resolved device matches the **overridden** value from
`.lufs-recorder.toml`, not whatever your global config specifies — confirming the override applies
from a real working directory, not just the sandbox's synthetic filesystem (which was already
verified — this just re-confirms on a real disk/real cwd resolution).

## Test 6 — live `serve` + the new API fields

```sh
./target/release/lrex serve --port 8777 &
sleep 1
curl -s http://127.0.0.1:8777/api/config | head -c 300; echo
```

**Expected:** valid JSON reflecting your config.

Now the synchronous-validation behavior added in v0.5 (never fired through a real running server +
a real HTTP request before):

```sh
curl -s -o /dev/null -w '%{http_code}\n' -X POST http://127.0.0.1:8777/api/record/start \
  -H 'Content-Type: application/json' \
  -d '{"device":"x","devices":[{"device":"y","tracks":[{"name":"a","channels":[1]}]}]}'
```

**Expected:** `400` (giving both `device` and `devices` is rejected before anything spawns).

Then a real single-device start/stop cycle over HTTP:

```sh
curl -s -X POST http://127.0.0.1:8777/api/record/start \
  -d '{"channels":"1-2","midi":"off","name":"http-test"}'
sleep 5
curl -s -X POST http://127.0.0.1:8777/api/record/stop
kill %1   # stop the serve process
```

**Expected:** `/stop` returns `{"stopped":true,"id":"...","take":{...,"verification":{"verified":true,...}}}`.
**Stop condition:** the 400 test returns anything other than 400, or the record/stop cycle doesn't
produce a verified take — report the exact HTTP status + body for whichever step failed.

## Test 7 — TUI live monitoring during a real take

`lrex tui` (added in v0.5.1) is a terminal monitor of `serve`'s existing status/stream API — no
new capture code, but its rendering of a **live** recording has only ever been tested against a
schema-accurate fixture, never a real take. This closes that gap. (The idle/unreachable/reconnect
states, and the masthead correctly naming itself under both binary names, are already verified in
the sandbox against a real running `serve` — see the PTY-driven checks referenced in the PR — so
this test is specifically about the one thing that couldn't be: real, moving meters.)

```sh
./target/release/lrex serve --port 8777 &
sleep 1
./target/release/lrex tui --port 8777 --theme lufs
```

With the TUI running, from a second terminal start a real take over the API (or use the `serve`
browser UI at `http://127.0.0.1:8777/` to start one):

```sh
curl -s -X POST http://127.0.0.1:8777/api/record/start \
  -d '{"channels":"1-2","midi":"off","name":"tui-watch-test"}'
```

**Expected:** within ~1s of starting, the TUI's masthead/body flips from the idle "waiting for a
recording" state to a live view: per-track meter bars that visibly move with real input level,
timecode advancing from `01:00:00.00` in real wall-clock time (not stuck, not racing ahead), and
the take's name shown in the masthead. Speak or play something into the input device partway
through and confirm the meters respond within a fraction of a second (the underlying SSE pushes
~12x/s).

Stop the take from the other terminal:

```sh
curl -s -X POST http://127.0.0.1:8777/api/record/stop
```

**Expected:** the TUI drops back to the idle state within ~1s (the same "waiting for a recording"
view Test 6 already exercises against a real server) rather than freezing on the last live frame.
Press `q` in the TUI to exit (expect exit `0`), then `kill %1` to stop `serve`.

**Also worth a quick look while it's running:** switch `--theme` (rerun with `catppuccin` and
`mono`) and confirm the meters/state are equally readable in each — the `mono` theme in particular
should never rely on color alone (every state also carries a distinct glyph: `○◐✓✗⊘`).

**Bonus, not required:** the TUI also has a staleness signal — if it's connected but hasn't heard
from `serve` in 3+ seconds it shows `⊘ stale` in the masthead. This should only ever be
observable if `serve` itself hangs (e.g. `kill -STOP` the `serve` process briefly, then
`kill -CONT` it); it should **not** appear during normal idle reconnect cycling or normal
recording. Not gating — this is a genuinely rare path to hit, mention it in your report only if
you happen to see it unexpectedly.

**Stop condition:** the TUI hangs instead of showing "waiting for a recording" after `/stop`;
meters never move despite real input; timecode drifts or resets; `⊘ stale` appears during normal
operation (that would mean the 3s threshold is too tight for real network/scheduling jitter).
Report the exact `--theme` used, a description (or screenshot/terminal recording, if easy) of what
rendered, and whether `q` still exited cleanly.

## Reporting back

For **each** test: the exact command run, full console/JSON output (or the relevant excerpt for
long output), the exit code, and a PASS / FAIL / PARTIAL call. If anything deviates from
"Expected," gather what evidence you reasonably can (the full `take.json`, the exact error text)
but stop short of patching source — hand the findings back (Slack `##process-registries`,
@-mention Ciani) so any real fix goes through a reviewed PR rather than an unreviewed hotfix on a
tool Daniel records with daily.
