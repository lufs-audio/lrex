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
//!   POST /api/verify           -> { id } | { dir } -> manifest + verification
//!   POST /api/record/start     -> { device?, tracks?|channels?, midi?, rate?, bit_depth?, name? }
//!   GET  /api/record/status    -> { recording, name?, elapsed_s? }
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
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

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
}

pub fn serve(
    cfg: Config,
    config_path: Option<PathBuf>,
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
    });

    let addr = format!("127.0.0.1:{port}");
    let listener =
        TcpListener::bind(&addr).map_err(|e| ExitError::Other(anyhow!("binding {addr}: {e}")))?;
    let url = format!("http://{addr}/");
    if json_out {
        println!("{}", json!({ "serving": url }));
    } else {
        eprintln!("lufs-recorder: control UI + API at {url}  (Ctrl-C to stop)");
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
    let path = raw_path.split('?').next().unwrap_or("/").to_string();

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

    let (status, ctype, payload) = route(&state, &method, &path, &body);
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Content-Type\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    writer.write_all(header.as_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
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
        ("GET", p) if p.starts_with("/api/takes/") => {
            let id = p.trim_start_matches("/api/takes/");
            wrap(api_take(state, id))
        }
        _ => err_json("404 Not Found", format!("no route for {method} {path}")),
    }
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
    if id.is_empty() || id.contains("..") || id.contains('/') {
        anyhow::bail!("invalid take id");
    }
    let dir = state.out_dir.join(id);
    let m = read_manifest(&dir).ok_or_else(|| anyhow!("no take.json in {}", dir.display()))?;
    Ok(serde_json::to_value(m)?)
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
            return json!({ "recording": false, "last": name, "note": "record process exited on its own" });
        }
        return json!({ "recording": true, "name": r.name, "elapsed_s": r.started.elapsed().as_secs_f64() });
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

    let mut args: Vec<String> = vec!["record".to_string()];
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

    let child = Command::new(&state.exe)
        .args(&args)
        .spawn()
        .context("spawning record process")?;

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
