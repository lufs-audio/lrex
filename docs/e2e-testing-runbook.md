# End-to-End Verification Runbook — lufs-recorder (`lrex`)

> **Target Audience:** Autonomous coding agents (via Pi, OpenCode, Jules) or human verification engineers executing exhaustive hardware/software validation of `lufs-recorder` (`lrex`).
> **Contract Authority:** `CONTRACT.md`, `AGENTS.md`, and `docs/specs/multi-device-and-voice-call-profiles/`.

---

## 1. Operating Directives & Agent Execution Rules

When an agent executes this runbook:
1. **Never scrape human CLI text:** Always supply `--json` to `devices`, `record --dry-run`, `verify`, and `selftest`. Parse the JSON structure and evaluate explicit exit codes.
2. **Exit Code Semantics are Absolute:**
   - `0`: Success & take contract verified (`take.json` has `verified: true`).
   - `2`: Bad usage / argument syntax.
   - `3`: Audio device or MIDI port unavailable.
   - `4`: Format unsupported (channel count, rate mismatch across devices).
   - `5`: Capture ran but failed contract (`xruns > 0`, silent audio, corrupted header).
   - `6`: Interrupted / cancelled before complete take.
3. **Capture Evidence on Failure:** If any stop condition is met, do not attempt heuristic monkey-patching. Capture:
   - Full CLI / HTTP response stdout and stderr.
   - Associated `take.json` manifest.
   - OS audio device snapshot (`lrex devices --json`).
4. **Environment Cleanliness:** Run tests using throwaway directories under `/tmp/lrex-e2e-*` or clean timestamped take folders. Do not overwrite user config `~/.config/lufs-recorder/config.toml` unless explicitly instructed.

---

## 2. Pre-Flight: Build & Zero-Hardware Baseline

### Step 2.1 — Build Verification & Clean Workspace
```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --verbose
```
- **Expected:** All tests pass (55/55 minimum), clippy produces zero warnings under `-D warnings`, format check is clean.
- **Stop Condition:** Any compilation, lint, or unit test failure.

### Step 2.2 — Dual Binary Alias Sanity
Verify both `lrex` and `lufs-recorder` executables report identical version numbers and customize usage strings to match invocation binary name.
```sh
cargo build --release
./target/release/lrex --version
./target/release/lufs-recorder --version
./target/release/lrex --help | head -n 3
./target/release/lufs-recorder --help | head -n 3
```
- **Expected:** Both report current version (e.g. `0.5.1`). `lrex --help` prints `Usage: lrex [OPTIONS] <COMMAND>`; `lufs-recorder --help` prints `Usage: lufs-recorder [OPTIONS] <COMMAND>`.
- **Stop Condition:** Binary names mismatched or incorrect usage header.

### Step 2.3 — In-Process Selftest (WAV, MIDI & Clock Math)
```sh
./target/release/lrex selftest --json
```
- **Expected:** Exit code `0`. JSON output contains:
  ```json
  {
    "ok": true,
    "tests_run": 5,
    "failures": []
  }
  ```
- **Stop Condition:** Exit code `!= 0` or `ok: false`.

### Step 2.4 — Project-Scoped Config Layering
```sh
TEST_DIR=$(mktemp -d /tmp/lrex-config-test-XXXXXX)
cd "$TEST_DIR"
cat > .lufs-recorder.toml <<'EOF'
rate = 44100
bit_depth = 16
EOF
/full/path/to/target/release/lrex record --dry-run --channels 1-2 --midi off --json
rm -rf "$TEST_DIR"
```
- **Expected:** Exit code `0`. Dry-run JSON reflects `rate_hz: 44100` and `bit_depth: "16"`.
- **Stop Condition:** Overridden fields do not match project `.lufs-recorder.toml`.

---

## 3. Hardware Discovery & Single-Device Capture Matrix

### Step 3.1 — Audio & MIDI Device Enumeration
```sh
./target/release/lrex devices --json
```
- **Expected:** Exit code `0`. JSON payload containing `audio_input_devices[]` (with `name`, `channels`, `default_rate`, `supported_rates`, `default_sample_format`) and `midi_input_ports[]`.
- **Action:** Record available device names and maximum channels for subsequent steps.

### Step 3.2 — Pre-Flight Dry Run
```sh
./target/release/lrex record --dry-run --device "<DEVICE_NAME>" --channels 1-2 --midi off --json
```
- **Expected:** Exit code `0`. JSON output outlines resolved device, channels, buffer sizes, sample format, and predicted output track paths without creating files on disk.

### Step 3.3 — Baseline Single-Device Capture (WAV 24-bit & 16-bit)
```sh
# 24-bit Capture
./target/release/lrex record \
  --device "<DEVICE_NAME>" \
  --channels 1-2 \
  --bit-depth 24 \
  --duration 5 \
  --midi off \
  --name "baseline-24bit" \
  --json

# 16-bit Capture
./target/release/lrex record \
  --device "<DEVICE_NAME>" \
  --channels 1-2 \
  --bit-depth 16 \
  --duration 5 \
  --midi off \
  --name "baseline-16bit" \
  --json
```
- **Expected:** Exit code `0`. Output directory created under `~/Samples/sampleLibrary/lufs-recorder/<timestamp>_<name>/`.
- **Verification Criteria:**
  - Inspect `take.json`: `verification.verified == true`.
  - `captured.xruns == 0`.
  - `captured.duration_s` within 10% of requested 5s.
  - WAV header confirms 24-bit or 16-bit PCM de-interleaved stereo.
- **Stop Condition:** Exit `!= 0`, `verified: false`, or `xruns > 0`.

### Step 3.4 — Multi-Track Single-Device Capture
```sh
./target/release/lrex record \
  --device "<DEVICE_NAME>" \
  --track "mic=1" \
  --track "aux=2" \
  --duration 5 \
  --midi off \
  --name "multitrack-single-dev" \
  --json
```
- **Expected:** Exit code `0`. Two discrete mono WAV files created (`mic.wav`, `aux.wav`). `take.json` lists both tracks mapped to the same device. Both tracks pass `audio_decodes` and `no_silence` checks.

---

## 4. Multi-Device Concurrent Capture & Fault Attribution

*Prerequisite: System exposes at least two distinct audio input devices (e.g. built-in mic + USB interface, or two virtual loopback interfaces like BlackHole / snd-aloop).*

### Step 4.1 — Multi-Device Concurrent Take
```sh
./target/release/lrex record \
  --device-track "<DEV_A>:track_a=1,2" \
  --device-track "<DEV_B>:track_b=1,2" \
  --duration 8 \
  --name "multidevice-clean" \
  --json
```
- **Expected:**
  - Exit code `0`.
  - Directory contains `track_a.wav` and `track_b.wav`.
  - `take.json` schema v3:
    - `captured.devices` lists both device names.
    - `captured.xruns_by_device` contains zero xruns for each device.
    - `verification.checks[]` contains per-device verification entries (`device_no_xruns`, `device_rate_matches`, `device_bit_depth_matches`, `device_audio_decodes`, `device_files_exist_nonempty`) for both `<DEV_A>` and `<DEV_B>`.
- **Stop Condition:** Exit code `!= 0` or missing per-device verification checks.

### Step 4.2 — Multi-Device Sample Rate Mismatch Rejection
Force a mismatch where Device A and Device B cannot reconcile a common sample rate (or specify `--rate` unsupported by one device).
```sh
./target/release/lrex record \
  --device-track "<DEV_A>:track_a=1,2" \
  --device-track "<DEV_B>:track_b=1,2" \
  --rate 192000 \
  --duration 3 \
  --json
```
*(Assuming one device does not support 192 kHz)*
- **Expected:** Fails fast with exit code `4` (`FormatUnsupported`). JSON error message explicitly identifies conflicting rate requirements without capturing corrupted audio.

---

## 5. Named Profiles & Wall-Clock Auto-Stop Timing

### Step 5.1 — Configuration Profile Setup
Append a test profile to `~/.config/lufs-recorder/config.toml` (or use local `.lufs-recorder.toml`):
```toml
[profiles.e2e-quick]
auto_stop_minutes = 0.1
buffer_minutes = 0.05
```
*Note: Formula is `(auto_stop_minutes + 2 * buffer_minutes) * 60` = `(0.1 + 0.1) * 60` = 12.0 seconds.*

### Step 5.2 — Real Wall-Clock Auto-Stop Execution
```sh
START_TIME=$(date +%s)
./target/release/lrex record \
  --channels 1-2 \
  --midi off \
  --profile e2e-quick \
  --name "profile-auto-stop" \
  --json
EXIT_CODE=$?
END_TIME=$(date +%s)
ELAPSED=$((END_TIME - START_TIME))
```
- **Expected:**
  - Exit code `0` without manual Ctrl-C intervention.
  - Total wall-clock elapsed time `ELAPSED` is between 11 and 14 seconds.
  - `take.json` reports `requested.duration_s: 12.0`.
- **Stop Condition:** Command fails to terminate automatically within 25 seconds or exit code is non-zero.

---

## 6. Contract Violation & Fault Injection Matrix

| Test Case | Command Execution | Expected Exit Code | Expected JSON Error / Result |
|---|---|---|---|
| **Bad Usage** | `lrex record --channels "0" --json` | `2` | `{"error":"bad_usage", ...}` |
| **Reversed Range** | `lrex record --channels "4-2" --json` | `2` | `{"error":"bad_usage", ...}` |
| **Unknown Device** | `lrex record --device "NonExistentDevice123" --json` | `3` | `{"error":"device_unavailable", ...}` |
| **Unsupported Channels** | `lrex record --channels "1-99" --json` (on 2ch device) | `4` | `{"error":"format_unsupported", ...}` |
| **Mutually Exclusive CLI** | `lrex record --device "A" --device-track "B:t=1" --json` | `2` | Clap argument collision |
| **Mutually Exclusive Profile**| `lrex record --profile "e2e-quick" --duration 10 --json` | `2` | Clap argument collision |

---

## 7. HTTP API & Server-Sent Events (SSE) Suite

### Step 7.1 — Launch Background Server
```sh
./target/release/lrex serve --port 8777 &
SERVER_PID=$!
sleep 1
```

### Step 7.2 — API Information & Selftest Endpoints
```sh
curl -s -f http://127.0.0.1:8777/api/config | jq .
curl -s -f http://127.0.0.1:8777/api/devices | jq .
curl -s -f http://127.0.0.1:8777/api/selftest | jq .
```
- **Expected:** All HTTP responses return `200 OK` with valid JSON schemas.

### Step 7.3 — Synchronous Validation Gate
```sh
# Conflict: both device and devices specified
STATUS_CONFLICT=$(curl -s -o /dev/null -w '%{http_code}' -X POST http://127.0.0.1:8777/api/record/start \
  -H 'Content-Type: application/json' \
  -d '{"device":"a","devices":[{"device":"b","tracks":[{"name":"t","channels":[1]}]}]}')
test "$STATUS_CONFLICT" = "400"
```
- **Expected:** HTTP Status `400 Bad Request`.

### Step 7.4 — HTTP Record & SSE Live Stream Cycle
```sh
# 1. Start recording
curl -s -X POST http://127.0.0.1:8777/api/record/start \
  -H 'Content-Type: application/json' \
  -d '{"channels":"1-2","midi":"off","name":"api-e2e-take"}' | jq .

# 2. Inspect active status
curl -s http://127.0.0.1:8777/api/record/status | jq .
# Expected: "recording": true, levels[] populated

# 3. Sample SSE stream (~2 seconds)
curl -s -N http://127.0.0.1:8777/api/record/stream | head -n 20

# 4. Stop recording
STOP_RESP=$(curl -s -X POST http://127.0.0.1:8777/api/record/stop)
echo "$STOP_RESP" | jq .
```
- **Expected:**
  - `/api/record/start` returns `{"started": true, "name": "api-e2e-take"}`.
  - `/api/record/status` shows `recording: true`, increasing frame counts, and active `levels[]`.
  - SSE stream emits `event: progress` frames with `levels[]`, signed `wave[]` samples, and zero xruns.
  - `/api/record/stop` returns `stopped: true` and a complete verified take manifest.

### Step 7.5 — Take Inspection Endpoints
```sh
TAKE_ID=$(echo "$STOP_RESP" | jq -r .id)
curl -s -f "http://127.0.0.1:8777/api/takes/$TAKE_ID" | jq .
curl -s -f "http://127.0.0.1:8777/api/takes/$TAKE_ID/waveform?track=track-01-02.wav&buckets=50" | jq .
```
- **Expected:** `200 OK` with valid take manifest and calculated peak envelope array.

### Step 7.6 — Tear Down Server
```sh
kill $SERVER_PID
```

---

## 8. TUI Live Monitor Interactive & Reconnect Suite

### Step 8.1 — Unreachable & Reconnect State
1. Launch `lrex tui --port 8777` when `serve` is **not** running.
   - **Expected:** Masthead displays `127.0.0.1:8777 · unreachable`. Body explains `cannot reach 127.0.0.1:8777` and prompts to start `lrex serve`.
2. Start `lrex serve --port 8777 &` in another terminal.
   - **Expected:** Within ~1s, TUI automatically reconnects without user input and transitions to `127.0.0.1:8777 · idle`.

### Step 8.2 — Theme Matrix & WCAG 1.4.1 Compliance Check
Run TUI against active server under each theme:
- `lrex tui --theme lufs` (Brand teal/gold/rust accents).
- `lrex tui --theme catppuccin` (Catppuccin Mocha palette).
- `lrex tui --theme mono` (Strict accessibility: no color hue; levels read via bar length and numerical dBFS; status states explicit via words like `REC` / `idle`).

### Step 8.3 — Live Take Monitoring
1. Keep `lrex tui` running in Terminal 1.
2. Trigger take in Terminal 2:
   ```sh
   curl -s -X POST http://127.0.0.1:8777/api/record/start -d '{"channels":"1-2","midi":"off","name":"tui-live-check"}'
   ```
   - **Expected:** Masthead switches to `● REC` (bold red dot in color themes, bold text in mono), take name displayed, timecode starts at `01:00:00.00` and ticks forward in real seconds, per-track meter bars respond dynamically to incoming audio.
3. Stop take:
   ```sh
   curl -s -X POST http://127.0.0.1:8777/api/record/stop
   ```
   - **Expected:** TUI returns to idle "waiting for a recording to start…" state within 1s.
4. Press `q` or `Esc` to exit cleanly (exit code `0`).

---

## 9. Verification Artifacts & Output Checklist

Every completed execution of this runbook must produce a structured artifact report formatted like `docs/verification-report-v0.5.md`:
1. **Header:** Date, commit SHA, host OS, toolchain version, audio backend (ALSA / CoreAudio).
2. **Device Inventory:** Complete JSON dump of `lrex devices --json`.
3. **Execution Table:** Pass/Fail status for all 8 test sections.
4. **Sample Manifests:** Raw `take.json` from multi-device capture and auto-stop profile takes.
5. **Certification Statement:** Explicit sign-off that contract verification (`verified: true`, `xruns == 0`) was upheld across all tested surfaces.
