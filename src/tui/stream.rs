//! A minimal, hand-rolled SSE client for `GET /api/record/stream` — the TUI's
//! only connection to the engine. Uses `std::net::TcpStream` directly rather
//! than an HTTP client crate, matching `server.rs`'s own "no web framework,
//! stay self-contained" choice for the same endpoint on the other end.
//!
//! Real, verified behavior this client is built against (confirmed by
//! reading `server.rs::stream_record` AND by starting a real `serve` process
//! in a sandbox with zero audio hardware and curling the endpoint directly):
//! while idle, the stream sends exactly one `data: {"recording":false}`
//! frame and then the connection closes -- it is NOT a persistent heartbeat.
//! A client that assumes an open connection stays open will silently stop
//! updating the moment a take ends (or if it connects before one starts).
//! The fix is architectural, not a workaround: treat every disconnect as
//! normal, and reconnect on a short timer for as long as the TUI is open.
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

#[derive(Debug)]
pub enum StreamEvent {
    /// The HTTP connection + SSE headers succeeded.
    Connected,
    /// One parsed `data: {...}` frame.
    Frame(Value),
    /// The connection closed (expected constantly while idle -- see module
    /// doc comment). Not an error; the reader loop reconnects on its own.
    Disconnected,
    /// The TCP connection itself could not be established (e.g. no `serve`
    /// process running at all). Distinct from `Disconnected` so the UI can
    /// say "can't reach host:port" instead of "waiting for a recording".
    ConnectError(String),
}

const RECONNECT_DELAY: Duration = Duration::from_millis(700);

/// Spawns a background thread that connects to `http://{host}:{port}/api/record/stream`,
/// forwards every event over the returned channel, and reconnects forever
/// (until the process exits) whenever the connection ends for any reason.
pub fn spawn(host: String, port: u16) -> Receiver<StreamEvent> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || loop {
        match read_once(&host, port, &tx) {
            Ok(()) => {
                let _ = tx.send(StreamEvent::Disconnected);
            }
            Err(e) => {
                let _ = tx.send(StreamEvent::ConnectError(e.to_string()));
            }
        }
        thread::sleep(RECONNECT_DELAY);
    });
    rx
}

fn read_once(host: &str, port: u16, tx: &mpsc::Sender<StreamEvent>) -> std::io::Result<()> {
    let mut stream = TcpStream::connect((host, port))?;
    // A read timeout so a genuinely wedged server (not just an idle-and-closed
    // one, which closes cleanly on its own) doesn't block this thread forever
    // and leave the reconnect loop stuck.
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;

    let request = format!(
        "GET /api/record/stream HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes())?;

    let mut reader = BufReader::new(stream);

    // Consume the HTTP status line + headers up to the blank line. Don't
    // bother parsing the status code: a non-SSE response simply won't match
    // "data: " lines below and the loop falls through to a clean disconnect.
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(()); // closed before headers finished
        }
        if line.trim().is_empty() {
            break;
        }
    }
    let _ = tx.send(StreamEvent::Connected);

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break; // server closed the connection -- expected, see doc comment
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if let Some(json_str) = trimmed.strip_prefix("data: ") {
            if let Ok(v) = serde_json::from_str::<Value>(json_str) {
                if tx.send(StreamEvent::Frame(v)).is_err() {
                    return Ok(()); // receiver gone (TUI exiting) -- stop quietly
                }
            }
        }
        // Blank keep-alive lines between SSE events are just skipped.
    }
    Ok(())
}
