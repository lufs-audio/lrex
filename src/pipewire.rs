//! PipeWire application-stream isolation (`record --app <name>`, Linux only).
//!
//! `--app` needs two distinct capabilities:
//!
//! 1. **Find** the running application's PipeWire playback node, matched
//!    case-insensitively against `application.name` / `application.process.binary`.
//!    That is implemented here against `pw-dump`'s JSON graph, and the matching
//!    itself is a pure function ([`match_playback_node`]) unit-tested against
//!    fabricated `pw-dump` output — verifiable with no live graph.
//! 2. **Capture exclusively** from that node. This requires a PipeWire-native
//!    capture backend that links a capture stream to the node's output ports;
//!    the cpal/ALSA backend this tool otherwise uses cannot address a PipeWire
//!    node at all. That backend is **not** part of this build, so `record::run`
//!    fails honestly by name (exit 70) rather than capturing the system mix and
//!    mislabelling it "isolated" — see the module note in `record.rs`.
//!
//! Everything below is therefore either pure/testable (the matcher) or an
//! honest, explicitly-typed failure naming the absent capability.

use serde_json::Value;

/// Why `--app` capture could not proceed. Each variant carries the exit-code
/// intent of its category (see `record::app_capture_exit`).
#[allow(dead_code)] // `ToolFailure` is only constructed on a Linux/PipeWire host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCaptureError {
    /// Not a Linux/PipeWire host: macOS/Windows, or a Linux host with no live
    /// PipeWire session. Maps to exit 4 (unsupported feature here).
    Unsupported(String),
    /// PipeWire is running but no active playback stream matched the requested
    /// application. Maps to exit 3 (unavailable source).
    NoMatchingStream(String),
    /// PipeWire tooling ran but failed. Maps to exit 4.
    ToolFailure(String),
}

impl std::fmt::Display for AppCaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppCaptureError::Unsupported(m)
            | AppCaptureError::NoMatchingStream(m)
            | AppCaptureError::ToolFailure(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for AppCaptureError {}

/// The exact message the contract specifies for a non-Linux / no-PipeWire host.
pub const UNSUPPORTED_MSG: &str = "--app is currently supported on Linux systems running PipeWire.";

/// Whether a live PipeWire session is present. Linux only: the session exposes
/// a `pipewire-0` socket under `$XDG_RUNTIME_DIR`. Always `false` elsewhere.
pub fn pipewire_available() -> bool {
    #[cfg(target_os = "linux")]
    let available = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => std::path::Path::new(&dir).join("pipewire-0").exists(),
        _ => false,
    };
    #[cfg(not(target_os = "linux"))]
    let available = false;
    available
}

/// Resolve the PipeWire node id of an application's active audio playback
/// stream.
///
/// On Linux with a live session: inspects `pw-dump`'s JSON graph. On any other
/// host (or a Linux host with no live PipeWire session): the explicit
/// [`UNSUPPORTED_MSG`] error, so the caller exits 4. With a live session but no
/// matching stream: a [`AppCaptureError::NoMatchingStream`], so the caller
/// exits 3 with the contract's exact message.
#[cfg(target_os = "linux")]
pub fn resolve_app_playback_node(app_name: &str) -> Result<u32, AppCaptureError> {
    if !pipewire_available() {
        return Err(AppCaptureError::Unsupported(UNSUPPORTED_MSG.to_string()));
    }
    let output = std::process::Command::new("pw-dump")
        .output()
        .map_err(|e| {
            AppCaptureError::ToolFailure(format!(
                "could not run `pw-dump` to inspect the PipeWire graph: {e}; {UNSUPPORTED_MSG}"
            ))
        })?;
    if !output.status.success() {
        return Err(AppCaptureError::ToolFailure(format!(
            "`pw-dump` failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let graph: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| AppCaptureError::ToolFailure(format!("parsing `pw-dump` JSON: {e}")))?;
    match_playback_node(&graph, app_name).ok_or_else(|| {
        AppCaptureError::NoMatchingStream(format!(
            "No active audio stream found for application '{app_name}'. \
             Ensure the application is playing audio and retry."
        ))
    })
}

#[cfg(not(target_os = "linux"))]
pub fn resolve_app_playback_node(_app_name: &str) -> Result<u32, AppCaptureError> {
    Err(AppCaptureError::Unsupported(UNSUPPORTED_MSG.to_string()))
}

/// Pure matcher over `pw-dump` JSON: the node id of the first audio playback
/// stream whose `application.name` or `application.process.binary` contains
/// `app_name` (case-insensitive substring), or `None`.
pub fn match_playback_node(graph: &Value, app_name: &str) -> Option<u32> {
    let needle = app_name.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    for node in graph.as_array()? {
        if !is_audio_playback_stream(node) {
            continue;
        }
        let Some(props) = node.get("info").and_then(|i| i.get("props")) else {
            continue;
        };
        let matched = ["application.name", "application.process.binary"]
            .iter()
            .any(|key| {
                props
                    .get(key)
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_lowercase().contains(&needle))
                    .unwrap_or(false)
            });
        if matched {
            if let Some(id) = node.get("id").and_then(|i| i.as_u64()) {
                return Some(id as u32);
            }
        }
    }
    None
}

/// A `pw-dump` entry is an audio playback stream iff its `media.class` is
/// `Stream/Output/Audio` (capture/duplex/monitor nodes are not the app's output).
fn is_audio_playback_stream(node: &Value) -> bool {
    node.get("info")
        .and_then(|i| i.get("props"))
        .and_then(|p| p.get("media.class"))
        .and_then(|v| v.as_str())
        == Some("Stream/Output/Audio")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> Value {
        serde_json::json!([
            { "id": 30, "type": "PipeWire:Interface:Node",
              "info": { "props": { "media.class": "Audio/Sink", "application.name": "Brave" } } },
            { "id": 41, "type": "PipeWire:Interface:Node",
              "info": { "props": { "media.class": "Stream/Input/Audio",
                                   "application.name": "Ardour" } } },
            { "id": 52, "type": "PipeWire:Interface:Node",
              "info": { "props": { "media.class": "Stream/Output/Audio",
                                   "application.name": "Brave",
                                   "application.process.binary": "brave" } } },
            { "id": 63, "type": "PipeWire:Interface:Node",
              "info": { "props": { "media.class": "Stream/Output/Audio",
                                   "application.name": "Spotify" } } }
        ])
    }

    #[test]
    fn matches_application_name_case_insensitively() {
        assert_eq!(match_playback_node(&graph(), "brave"), Some(52));
    }

    #[test]
    fn matches_process_binary_as_fallback() {
        assert_eq!(match_playback_node(&graph(), "spotify"), Some(63));
    }

    #[test]
    fn ignores_non_playback_and_sink_nodes() {
        // "Ardour" is a Stream/Input/Audio (capture) node — never a match.
        assert_eq!(match_playback_node(&graph(), "ardour"), None);
        // A sink carrying application.name "Brave" is not a playback stream.
        let only_sink = serde_json::json!([
            { "id": 1, "info": { "props": { "media.class": "Audio/Sink",
                                            "application.name": "Brave" } } }
        ]);
        assert_eq!(match_playback_node(&only_sink, "brave"), None);
    }

    #[test]
    fn unknown_application_is_none() {
        assert_eq!(match_playback_node(&graph(), "Nonexistent"), None);
    }

    #[test]
    fn empty_query_is_none() {
        assert_eq!(match_playback_node(&graph(), "  "), None);
    }

    #[test]
    fn non_array_graph_is_none() {
        assert_eq!(
            match_playback_node(&serde_json::json!({"oops": true}), "brave"),
            None
        );
    }

    #[test]
    fn resolve_without_a_live_session_reports_unsupported_by_name() {
        if !pipewire_available() {
            let err = resolve_app_playback_node("Brave").unwrap_err();
            assert_eq!(
                err,
                AppCaptureError::Unsupported(UNSUPPORTED_MSG.to_string())
            );
            assert!(err.to_string().contains("Linux systems running PipeWire"));
        }
    }
}
