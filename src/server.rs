//! `serve` — a tiny local control surface + JSON API over the engine.
//!
//! This is the backend a browser frontend talks to. The browser can't do
//! reliable multichannel + MIDI *capture* (that's the Rust binary's job), so the
//! UI is a control surface: it lists devices, starts/stops takes, and shows the
//! verified results. The HTTP layer is hand-rolled on `std::net` (no web
//! framework) to keep the tool self-contained and local-first.
//!
//! Endpoint contract (stable — this is what a real frontend builds against):
//!   GET  /                     -> control UI (embedded, or --frontend dir)
//!   GET  /api/devices          -> { audio_input_devices, midi_input_ports }
//!   GET  /api/config           -> { config_path, config, out_dir }
//!   GET  /api/selftest         -> { passed, checks }
//!   GET  /api/takes            -> [ { id, take_id, created, verified, ... } ]
//!   GET  /api/takes/<id>       -> full take manifest
//!   GET  /api/takes/<id>/notes -> parsed MIDI notes [{channel,key,vel,start_s,dur_s}]
//!   GET  /api/takes/<id>/waveform?track=<file>&buckets=N -> peak envelope
//!   GET  /api/takes/<id>/file/<name> -> raw WAV/MIDI bytes (Web Audio / download)
//!   POST /api/verify           -> { id } | { dir } -> manifest + verification
//!   POST /api/record/start     -> { device?, tracks?|channels?, midi?, rate?, bit_depth?, name? }
//!   GET  /api/record/status    -> { recording, name?, elapsed_s?, frames?, xruns?, levels?,
//!                                     notes?, active?, midi_events? }
//!        (levels[]: per-track {name, peak_dbfs, rms_dbfs, wave[]}; wave[]: signed [-1,1]
//!         scope samples (~2400/s); notes[]: {key,vel} note-ons this frame; active[]: held
//!         MIDI keys — all live while recording)
//!   GET  /api/record/stream    -> SSE: pushes the same snapshot ~12x/s (live scope + keyboard)
//!   POST /api/record/stop      -> { stopped, id, take: manifest }
//!
//! Recording currently shells out to `lufs-recorder record` and stops it with
//! SIGINT (so the take finalizes + verifies exactly like the CLI). That's an
//! implementation detail behind the API; it can move in-process later without
//! changing a single endpoint.

use crate::config::Config;
use crate::error::ExitError;
use crate::manifest::Manifest;
use crate::{devices, fixture, midi, verify};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// The throwaway control UI, compiled into the binary. Override at runtime with
/// `serve --frontend <dir>` for live iteration (Amacher).
static FRONTEND_HTML: &str = include_str!("../frontend/index.html");

struct RecordState {
    child: Child,
    name: String,
    started: Instant,
}

struct AppState {
    cfg: Config,
    config_path: Option<PathBuf>,
    frontend_dir: Option<PathBuf>,
    out_dir: PathBuf,
    exe: PathBuf,
    rec: Mutex<Option<RecordState>>,
    /// Latest live progress snapshot ({elapsed_s, frames, xruns, levels[]})
    /// parsed from the record subprocess's NDJSON while recording.
    live: Arc<Mutex<Option<Value>>>,
}

pub fn serve(
    cfg: Config,
    config_path: Option<PathBuf>,
    host: String,
    port: u16,
    frontend_dir: Option<PathBuf>,
    json_out: bool,
) -> crate::error::Result<()> {
    let out_dir = cfg.out_path();
    let exe = std::env::current_exe().map_err(|e| ExitError::Other(e.into()))?;
    let state = Arc::new(AppState {
        cfg,
        config_path,
        frontend_dir,
        out_dir,
        exe,
        rec: Mutex::new(None),
        live: Arc::new(Mutex::new(None)),
    });

    let addr = format!("{host}:{port}");
    let listener =
        TcpListener::bind(&addr).map_err(|e| ExitError::Other(anyhow!("binding {addr}: {e}")))?;
    // For 0.0.0.0, point the user at a reachable URL (localhost) rather than the
    // wildcard bind address.
    let shown = if host == "0.0.0.0" {
        format!("http://localhost:{port}/  (also on your LAN IP:{port})")
    } else {
        format!("http://{addr}/")
    };
    if json_out {
        println!("{}", json!({ "serving": shown, "bind": addr }));
    } else {
        eprintln!("lufs-recorder: control UI + API at {shown}  (Ctrl-C to stop)");
        if host == "0.0.0.0" {
            eprintln!(
                "lufs-recorder: bound to 0.0.0.0 — reachable by other devices on your network."
            );
        }
    }

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let st = state.clone();
                std::thread::spawn(move || {
                    let _ = handle(st, s);
                });
            }
            Err(_) => continue,
        }
    }
    Ok(())
}

fn handle(state: Arc<AppState>, stream: TcpStream) -> Result<()> {
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut it = request_line.split_whitespace();
    let method = it.next().unwrap_or("").to_string();
    let raw_path = it.next().unwrap_or("/").to_string();
    let mut split = raw_path.splitn(2, '?');
    let path = split.next().unwrap_or("/").to_string();
    let query = split.next().unwrap_or("").to_string();

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let t = line.trim_end();
        if t.is_empty() {
            break;
        }
        if let Some(v) = t
            .strip_prefix("Content-Length:")
            .or_else(|| t.strip_prefix("content-length:"))
        {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body_buf = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body_buf)?;
    }
    let body = String::from_utf8_lossy(&body_buf).to_string();

    // Server-Sent Events stream for live monitoring — hijacks the connection and
    // pushes snapshots until recording stops or the client disconnects.
    if method == "GET" && path == "/api/record/stream" {
        return stream_record(&state, writer);
    }

    let (status, ctype, payload) = route(&state, &method, &path, &query, &body);
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Content-Type\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    writer.write_all(header.as_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

/// SSE live-monitoring stream. Pushes `{recording, elapsed_s, frames, xruns,
/// levels[{name,peak_dbfs,rms_dbfs,wave[]}], notes[], active[], midi_events}`
/// ~12×/s while recording, then a final `{recording:false}` and closes. `wave[]`
/// is signed [-1,1] scope samples (~2400/s) — a real oscilloscope trace.
fn stream_record(state: &AppState, mut writer: TcpStream) -> Result<()> {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
    writer.write_all(head.as_bytes())?;
    writer.flush()?;

    loop {
        // Recording state + elapsed, reaping a self-exited child.
        let (recording, elapsed) = {
            let mut guard = state.rec.lock().unwrap();
            match guard.as_mut() {
                Some(r) => {
                    if let Ok(Some(_)) = r.child.try_wait() {
                        *guard = None;
                        (false, 0.0)
                    } else {
                        (true, r.started.elapsed().as_secs_f64())
                    }
                }
                None => (false, 0.0),
            }
        };

        let ev = if recording {
            live_event(state.live.lock().unwrap().clone(), elapsed, None)
        } else {
            json!({ "recording": false })
        };

        if writer
            .write_all(format!("data: {ev}\n\n").as_bytes())
            .is_err()
        {
            break;
        }
        if writer.flush().is_err() {
            break;
        }
        if !recording {
            break;
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    Ok(())
}

/// Build a "recording" live event by passing the whole progress snapshot
/// through (frames, xruns, levels, notes, active, midi_events, …) and stamping
/// `recording`/`elapsed_s`/`name`. Future stream fields flow with no changes here.
fn live_event(snapshot: Option<Value>, elapsed: f64, name: Option<&str>) -> Value {
    let mut out = snapshot.unwrap_or_else(|| json!({}));
    if !out.is_object() {
        out = json!({});
    }
    let obj = out.as_object_mut().expect("object");
    obj.remove("event");
    obj.insert("recording".to_string(), json!(true));
    obj.insert("elapsed_s".to_string(), json!(elapsed));
    if let Some(n) = name {
        obj.insert("name".to_string(), json!(n));
    }
    out
}

fn ok_json(v: Value) -> (String, &'static str, Vec<u8>) {
    (
        "200 OK".to_string(),
        "application/json",
        serde_json::to_vec_pretty(&v).unwrap_or_default(),
    )
}

fn err_json(code: &str, msg: String) -> (String, &'static str, Vec<u8>) {
    (
        code.to_string(),
        "application/json",
        serde_json::to_vec(&json!({ "error": msg })).unwrap_or_default(),
    )
}

fn route(
    state: &AppState,
    method: &str,
    path: &str,
    query: &str,
    body: &str,
) -> (String, &'static str, Vec<u8>) {
    if method == "OPTIONS" {
        return ("204 No Content".to_string(), "text/plain", Vec::new());
    }

    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => serve_frontend(state),
        ("GET", "/api/devices") => wrap(api_devices()),
        ("GET", "/api/config") => wrap(api_config(state)),
        ("GET", "/api/selftest") => wrap(api_selftest()),
        ("GET", "/api/takes") => wrap(Ok(api_takes(state))),
        ("GET", "/api/record/status") => wrap(Ok(api_status(state))),
        ("POST", "/api/record/start") => wrap(api_record_start(state, body)),
        ("POST", "/api/record/stop") => wrap(api_record_stop(state)),
        ("POST", "/api/verify") => wrap(api_verify(state, body)),
        // Take sub-resources: /api/takes/<id>[/notes | /waveform | /file/<name>]
        ("GET", p) if p.starts_with("/api/takes/") => {
            let rest = p.trim_start_matches("/api/takes/");
            let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
            match segs.as_slice() {
                [id] => wrap(api_take(state, id)),
                [id, "notes"] => wrap(api_take_notes(state, id)),
                [id, "waveform"] => wrap(api_take_waveform(state, id, query)),
                [id, "file", name] => serve_take_file(state, id, name),
                _ => err_json("404 Not Found", format!("no route for {method} {path}")),
            }
        }
        _ => err_json("404 Not Found", format!("no route for {method} {path}")),
    }
}

/// Reject anything that could escape the take directory.
fn safe_component(s: &str) -> Result<&str> {
    if s.is_empty() || s.contains("..") || s.contains('/') || s.contains('\\') {
        anyhow::bail!("invalid path component");
    }
    Ok(s)
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let mut it = kv.splitn(2, '=');
        if it.next()? == key {
            Some(it.next().unwrap_or("").to_string())
        } else {
            None
        }
    })
}

/// Turn a handler Result into an HTTP response.
fn wrap(r: Result<Value>) -> (String, &'static str, Vec<u8>) {
    match r {
        Ok(v) => ok_json(v),
        Err(e) => err_json("400 Bad Request", format!("{e:#}")),
    }
}

fn serve_frontend(state: &AppState) -> (String, &'static str, Vec<u8>) {
    if let Some(dir) = &state.frontend_dir {
        let index = dir.join("index.html");
        if let Ok(html) = std::fs::read(&index) {
            return ("200 OK".to_string(), "text/html; charset=utf-8", html);
        }
    }
    (
        "200 OK".to_string(),
        "text/html; charset=utf-8",
        FRONTEND_HTML.as_bytes().to_vec(),
    )
}

fn api_devices() -> Result<Value> {
    let audio = devices::list_input_devices()?;
    let midi_ports = midi::list_ports().unwrap_or_default();
    Ok(json!({ "audio_input_devices": audio, "midi_input_ports": midi_ports }))
}

fn api_config(state: &AppState) -> Result<Value> {
    Ok(json!({
        "config_path": state.config_path.as_ref().map(|p| p.display().to_string()),
        "config": state.cfg,
        "out_dir": state.out_dir.display().to_string(),
    }))
}

fn api_selftest() -> Result<Value> {
    let report = fixture::run_all()?;
    Ok(json!({ "passed": report.passed(), "checks": report.checks }))
}

fn read_manifest(dir: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(dir.join("take.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn api_takes(state: &AppState) -> Value {
    let mut takes: Vec<Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&state.out_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            if let Some(m) = read_manifest(&p) {
                takes.push(json!({
                    "id": p.file_name().and_then(|s| s.to_str()).unwrap_or_default(),
                    "take_id": m.take_id,
                    "created": m.created,
                    "verified": m.verification.verified,
                    "duration_s": m.captured.duration_s,
                    "xruns": m.captured.xruns,
                    "tracks": m.tracks.len(),
                    "midi": m.midi.is_some(),
                }));
            }
        }
    }
    // Newest first (created is ISO-8601, so lexical sort works).
    takes.sort_by(|a, b| {
        b["created"]
            .as_str()
            .unwrap_or_default()
            .cmp(a["created"].as_str().unwrap_or_default())
    });
    json!({ "out_dir": state.out_dir.display().to_string(), "takes": takes })
}

fn api_take(state: &AppState, id: &str) -> Result<Value> {
    let id = safe_component(id)?;
    let dir = state.out_dir.join(id);
    let m = read_manifest(&dir).ok_or_else(|| anyhow!("no take.json in {}", dir.display()))?;
    Ok(serde_json::to_value(m)?)
}

/// Parsed MIDI notes for a take — the data behind a real piano-roll / kslider
/// lane (start/duration in seconds), so the frontend never parses SMF itself.
fn api_take_notes(state: &AppState, id: &str) -> Result<Value> {
    let id = safe_component(id)?;
    let dir = state.out_dir.join(id);
    let manifest =
        read_manifest(&dir).ok_or_else(|| anyhow!("no take.json in {}", dir.display()))?;
    let mid_name = manifest
        .midi
        .as_ref()
        .map(|m| m.file.clone())
        .unwrap_or_else(|| "capture.mid".to_string());
    let mid_path = dir.join(&mid_name);
    if !mid_path.is_file() {
        return Ok(json!({ "id": id, "notes": [], "count": 0, "note": "no MIDI in this take" }));
    }
    let bytes =
        std::fs::read(&mid_path).with_context(|| format!("reading {}", mid_path.display()))?;
    let notes = crate::midi::parse_notes(&bytes)?;
    Ok(json!({
        "id": id,
        "count": notes.len(),
        "duration_s": manifest.captured.duration_s,
        "notes": notes,
    }))
}

/// A downsampled peak envelope for a track's WAV — a fast static waveform
/// without shipping/decoding the whole file in the browser. `?track=<file>&buckets=N`.
fn api_take_waveform(state: &AppState, id: &str, query: &str) -> Result<Value> {
    let id = safe_component(id)?;
    let dir = state.out_dir.join(id);
    let manifest =
        read_manifest(&dir).ok_or_else(|| anyhow!("no take.json in {}", dir.display()))?;

    let track = match query_param(query, "track") {
        Some(t) if !t.is_empty() => t,
        _ => manifest
            .tracks
            .first()
            .map(|t| t.file.clone())
            .ok_or_else(|| anyhow!("take has no tracks"))?,
    };
    let track = safe_component(&track)?;
    let buckets: usize = query_param(query, "buckets")
        .and_then(|b| b.parse().ok())
        .unwrap_or(800)
        .clamp(16, 8000);

    let wav_path = dir.join(track);
    let mut reader = hound::WavReader::open(&wav_path)
        .with_context(|| format!("opening {}", wav_path.display()))?;
    let spec = reader.spec();
    let ch = spec.channels.max(1) as usize;

    // Per-frame peak (max |sample| across channels), then bucketed.
    let mut frame_peaks: Vec<f32> = Vec::new();
    let mut cur: f32 = 0.0;
    let mut in_frame = 0usize;
    let denom = 2f64.powi((spec.bits_per_sample as i32 - 1).max(0));
    let push_sample = |v: f32, cur: &mut f32, in_frame: &mut usize, frame_peaks: &mut Vec<f32>| {
        *cur = cur.max(v.abs());
        *in_frame += 1;
        if *in_frame == ch {
            frame_peaks.push(*cur);
            *cur = 0.0;
            *in_frame = 0;
        }
    };
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for s in reader.samples::<f32>() {
                push_sample(s?, &mut cur, &mut in_frame, &mut frame_peaks);
            }
        }
        hound::SampleFormat::Int => {
            for s in reader.samples::<i32>() {
                push_sample(
                    (s? as f64 / denom) as f32,
                    &mut cur,
                    &mut in_frame,
                    &mut frame_peaks,
                );
            }
        }
    }

    let n = frame_peaks.len();
    let bucket_count = buckets.min(n.max(1));
    let mut peaks: Vec<f32> = Vec::with_capacity(bucket_count);
    if n > 0 {
        for b in 0..bucket_count {
            let start = b * n / bucket_count;
            let end = ((b + 1) * n / bucket_count).max(start + 1).min(n);
            let mut p = 0.0f32;
            for &v in &frame_peaks[start..end] {
                p = p.max(v);
            }
            peaks.push(p);
        }
    }

    Ok(json!({
        "id": id,
        "track": track,
        "channels": spec.channels,
        "sample_rate": spec.sample_rate,
        "frames": n,
        "buckets": peaks.len(),
        "peaks": peaks,
    }))
}

/// Serve a take's raw file (WAV/MIDI/JSON) so the browser can decode audio via
/// Web Audio (oscilloscope / spectrogram) or fetch the SMF directly.
fn serve_take_file(state: &AppState, id: &str, name: &str) -> (String, &'static str, Vec<u8>) {
    let resolved = (|| -> Result<(PathBuf, &'static str)> {
        let id = safe_component(id)?;
        let name = safe_component(name)?;
        let ctype: &'static str = if name.ends_with(".wav") {
            "audio/wav"
        } else if name.ends_with(".mid") {
            "audio/midi"
        } else if name.ends_with(".json") {
            "application/json"
        } else {
            "application/octet-stream"
        };
        Ok((state.out_dir.join(id).join(name), ctype))
    })();

    match resolved {
        Ok((path, ctype)) => match std::fs::read(&path) {
            Ok(bytes) => ("200 OK".to_string(), ctype, bytes),
            Err(_) => err_json("404 Not Found", format!("no file {}", path.display())),
        },
        Err(e) => err_json("400 Bad Request", format!("{e:#}")),
    }
}

fn api_verify(state: &AppState, body: &str) -> Result<Value> {
    #[derive(Deserialize, Default)]
    struct Req {
        id: Option<String>,
        dir: Option<String>,
    }
    let req: Req = if body.trim().is_empty() {
        Req::default()
    } else {
        serde_json::from_str(body).context("parsing verify body")?
    };
    let dir: PathBuf = match (req.id, req.dir) {
        (Some(id), _) if !id.is_empty() => {
            if id.contains("..") || id.contains('/') {
                anyhow::bail!("invalid take id");
            }
            state.out_dir.join(id)
        }
        (_, Some(d)) if !d.is_empty() => PathBuf::from(d),
        _ => anyhow::bail!("verify needs an 'id' or 'dir'"),
    };
    let (manifest, verification) = verify::verify_dir(&dir)?;
    Ok(json!({ "verified": verification.verified, "take": manifest }))
}

fn api_status(state: &AppState) -> Value {
    let mut guard = state.rec.lock().unwrap();
    if let Some(r) = guard.as_mut() {
        // If the record process died on its own (e.g. device error), don't leave
        // the UI stuck showing "recording".
        if let Ok(Some(_)) = r.child.try_wait() {
            let name = r.name.clone();
            *guard = None;
            *state.live.lock().unwrap() = None;
            return json!({ "recording": false, "last": name, "note": "record process exited on its own" });
        }
        // Pass the whole live snapshot through (peak/RMS, frames, xruns, wave,
        // notes, active, midi_events) so the UI can drive meters/scopes/keyboard.
        let elapsed = r.started.elapsed().as_secs_f64();
        let snapshot = state.live.lock().unwrap().clone();
        return live_event(snapshot, elapsed, Some(&r.name));
    }
    json!({ "recording": false })
}

fn api_record_start(state: &AppState, body: &str) -> Result<Value> {
    #[derive(Deserialize)]
    struct TrackReq {
        name: String,
        channels: Vec<u16>,
    }
    #[derive(Deserialize, Default)]
    struct StartReq {
        device: Option<String>,
        channels: Option<String>,
        tracks: Option<Vec<TrackReq>>,
        midi: Option<String>,
        rate: Option<u32>,
        bit_depth: Option<String>,
        name: Option<String>,
    }

    let mut guard = state.rec.lock().unwrap();
    if guard.is_some() {
        anyhow::bail!("already recording — stop the current take first");
    }
    let req: StartReq = if body.trim().is_empty() {
        StartReq::default()
    } else {
        serde_json::from_str(body).context("parsing record/start body")?
    };

    // `--json` so the child emits NDJSON progress (with per-track levels) that we
    // relay for live monitoring; the take still lands on disk for `stop` to read.
    let mut args: Vec<String> = vec!["--json".to_string(), "record".to_string()];
    if let Some(p) = &state.config_path {
        args.push("--config".to_string());
        args.push(p.display().to_string());
    }
    if let Some(d) = &req.device {
        args.push("--device".to_string());
        args.push(d.clone());
    }
    if let Some(m) = &req.midi {
        args.push("--midi".to_string());
        args.push(m.clone());
    }
    if let Some(r) = req.rate {
        args.push("--rate".to_string());
        args.push(r.to_string());
    }
    if let Some(b) = &req.bit_depth {
        args.push("--bit-depth".to_string());
        args.push(b.clone());
    }
    let name = req.name.clone().unwrap_or_else(|| "session".to_string());
    args.push("--name".to_string());
    args.push(name.clone());
    if let Some(tracks) = &req.tracks {
        for t in tracks {
            let chans = t
                .channels
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(",");
            args.push("--track".to_string());
            args.push(format!("{}={}", t.name, chans));
        }
    } else if let Some(ch) = &req.channels {
        args.push("--channels".to_string());
        args.push(ch.clone());
    }

    // Fresh monitoring state for this take.
    *state.live.lock().unwrap() = None;

    let mut child = Command::new(&state.exe)
        .args(&args)
        .stdout(Stdio::piped())
        .spawn()
        .context("spawning record process")?;

    // Reader thread: parse the child's NDJSON progress into the live snapshot.
    if let Some(stdout) = child.stdout.take() {
        let live = state.live.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(std::result::Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if v.get("event").and_then(|e| e.as_str()) == Some("progress") {
                        *live.lock().unwrap() = Some(v);
                    }
                }
            }
        });
    }

    *guard = Some(RecordState {
        child,
        name: name.clone(),
        started: Instant::now(),
    });
    Ok(json!({ "recording": true, "name": name }))
}

#[cfg(unix)]
fn interrupt(pid: u32) {
    // SIGINT so the child's Ctrl-C handler finalizes + verifies the take.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGINT);
    }
}

#[cfg(not(unix))]
fn interrupt(_pid: u32) {}

fn newest_take(out_dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for e in std::fs::read_dir(out_dir).ok()?.flatten() {
        let p = e.path();
        let take_json = p.join("take.json");
        if take_json.is_file() {
            let m = std::fs::metadata(&take_json)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            if best.as_ref().map(|(t, _)| m > *t).unwrap_or(true) {
                best = Some((m, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

fn api_record_stop(state: &AppState) -> Result<Value> {
    let mut rec = state
        .rec
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| anyhow!("not recording"))?;

    interrupt(rec.child.id());
    let _ = rec.child.wait();
    *state.live.lock().unwrap() = None;

    let take_dir = newest_take(&state.out_dir).ok_or_else(|| {
        anyhow!(
            "recording stopped but no take.json was found in {}",
            state.out_dir.display()
        )
    })?;
    let manifest = read_manifest(&take_dir)
        .ok_or_else(|| anyhow!("could not read take.json in {}", take_dir.display()))?;

    Ok(json!({
        "stopped": true,
        "id": take_dir.file_name().and_then(|s| s.to_str()).unwrap_or_default(),
        "take": manifest,
    }))
}
