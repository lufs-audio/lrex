//! The live meter/timecode/log TUI -- a `ratatui` client of `serve`'s own
//! `GET /api/record/stream` SSE endpoint. Deliberately a MONITOR, not a
//! control surface: it does not itself call `/api/record/start` or `/stop`.
//! Daniel's brief for this tool named exactly four things to show (a
//! cliamp-style level meter, a film-style timecode from 01:00:00.00, a
//! lazygit-style single-line log, a red REC dot) -- all display, nothing
//! about starting/stopping from the TUI itself. Recording is started the way
//! it already is today (`lrex record ...`, or the web UI's own controls);
//! this tool watches. That's a real, intentional scope line, not an
//! oversight -- see the crate-level docs for the follow-up if that's wrong.
use crate::tui::stream::{self, StreamEvent};
use crate::tui::theme::{Theme, ThemeKind};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};
use std::io::{self, Stdout};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
struct LevelInfo {
    name: String,
    peak_dbfs: f64,
    rms_dbfs: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Connection {
    Connecting,
    Connected,
    Unreachable,
}

struct AppState {
    theme: Theme,
    /// argv[0]'s basename (`lrex` or `lufs-recorder`) -- matches how every
    /// other part of this binary (help text, error messages, see
    /// `crate::invoked_name()`) names itself after however it was actually
    /// invoked, rather than hardcoding one alias in just this one surface.
    tool_name: String,
    host: String,
    port: u16,
    connection: Connection,
    recording: bool,
    take_name: Option<String>,
    elapsed_s: f64,
    frames: u64,
    xruns: u64,
    levels: Vec<LevelInfo>,
    active_midi: Vec<u8>,
    midi_events: u64,
    log: String,
    last_event_at: Instant,
}

impl AppState {
    fn new(host: String, port: u16, theme: Theme) -> Self {
        Self {
            theme,
            tool_name: crate::invoked_name(),
            host,
            port,
            connection: Connection::Connecting,
            recording: false,
            take_name: None,
            elapsed_s: 0.0,
            frames: 0,
            xruns: 0,
            levels: Vec::new(),
            active_midi: Vec::new(),
            midi_events: 0,
            log: "connecting…".to_string(),
            last_event_at: Instant::now(),
        }
    }

    /// Whether the stream has gone quiet for longer than a live connection
    /// should ever stay silent. Idle takes close the SSE connection after one
    /// frame (see `stream.rs`) and reconnect quickly, so this only trips for
    /// a genuinely wedged server sitting inside `stream.rs`'s 20s read
    /// timeout -- a real state, not a hypothetical, so it gets a real signal
    /// instead of the UI just looking frozen for up to 20s before
    /// `ConnectError` finally fires.
    fn is_stale(&self) -> bool {
        self.connection == Connection::Connected
            && self.last_event_at.elapsed() > Duration::from_secs(3)
    }

    fn masthead_meta(&self) -> String {
        let stale = if self.is_stale() { " · ⊘ stale" } else { "" };
        match self.connection {
            Connection::Unreachable => format!("{}:{} · unreachable", self.host, self.port),
            _ => match &self.take_name {
                Some(n) if self.recording => {
                    format!("{}:{} · take {n}{stale}", self.host, self.port)
                }
                _ => format!("{}:{} · idle{stale}", self.host, self.port),
            },
        }
    }

    fn apply_frame(&mut self, v: serde_json::Value) {
        self.last_event_at = Instant::now();
        self.recording = v
            .get("recording")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        if !self.recording {
            self.take_name = None;
            self.elapsed_s = 0.0;
            self.levels.clear();
            self.log = "waiting for a recording to start".to_string();
            return;
        }
        self.take_name = v
            .get("name")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.take_name.clone());
        self.elapsed_s = v
            .get("elapsed_s")
            .and_then(|x| x.as_f64())
            .unwrap_or(self.elapsed_s);
        self.frames = v
            .get("frames")
            .and_then(|x| x.as_u64())
            .unwrap_or(self.frames);
        self.xruns = v
            .get("xruns")
            .and_then(|x| x.as_u64())
            .unwrap_or(self.xruns);
        if let Some(levels) = v.get("levels").and_then(|x| x.as_array()) {
            self.levels = levels
                .iter()
                .map(|l| LevelInfo {
                    name: l
                        .get("name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("track")
                        .to_string(),
                    peak_dbfs: l
                        .get("peak_dbfs")
                        .and_then(|x| x.as_f64())
                        .unwrap_or(-120.0),
                    rms_dbfs: l.get("rms_dbfs").and_then(|x| x.as_f64()).unwrap_or(-120.0),
                })
                .collect();
        }
        if let Some(active) = v.get("active").and_then(|x| x.as_array()) {
            self.active_midi = active
                .iter()
                .filter_map(|x| x.as_u64().map(|n| n as u8))
                .collect();
        }
        self.midi_events = v
            .get("midi_events")
            .and_then(|x| x.as_u64())
            .unwrap_or(self.midi_events);
        self.log = format!(
            "take {} · {:.1}s · xruns {}",
            self.take_name.as_deref().unwrap_or("?"),
            self.elapsed_s,
            self.xruns
        );
    }
}

/// Film-convention timecode: starts at 01:00:00.00, never negative pre-roll.
fn format_timecode(elapsed_s: f64) -> String {
    let total_cs = ((3600.0 + elapsed_s.max(0.0)) * 100.0).round() as u64;
    let h = total_cs / 360_000;
    let m = (total_cs / 6_000) % 60;
    let s = (total_cs / 100) % 60;
    let cs = total_cs % 100;
    format!("{h:02}:{m:02}:{s:02}.{cs:02}")
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

pub fn run(host: String, port: u16, theme_kind: ThemeKind) -> crate::error::Result<()> {
    let guard =
        TerminalGuard::new().map_err(|e: io::Error| crate::error::ExitError::Other(e.into()))?;
    let rx = stream::spawn(host.clone(), port);
    let mut state = AppState::new(host, port, Theme::from_kind(theme_kind));
    event_loop(guard, &mut state, rx)
}

fn event_loop(
    mut guard: TerminalGuard,
    state: &mut AppState,
    rx: Receiver<StreamEvent>,
) -> crate::error::Result<()> {
    loop {
        guard
            .terminal
            .draw(|frame| draw(frame, state))
            .map_err(|e: io::Error| crate::error::ExitError::Other(e.into()))?;

        // Drain every pending stream event without blocking the key-poll below.
        loop {
            match rx.try_recv() {
                Ok(StreamEvent::Connected) => {
                    state.connection = Connection::Connected;
                }
                Ok(StreamEvent::Frame(v)) => {
                    state.connection = Connection::Connected;
                    state.apply_frame(v);
                }
                Ok(StreamEvent::Disconnected) => {
                    // Normal and frequent while idle (see stream.rs doc
                    // comment) -- do NOT treat this as an error state.
                    if state.connection != Connection::Unreachable {
                        state.connection = Connection::Connected;
                    }
                }
                Ok(StreamEvent::ConnectError(msg)) => {
                    state.connection = Connection::Unreachable;
                    state.log = format!("cannot reach {}:{} — {msg}", state.host, state.port);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        if event::poll(Duration::from_millis(120))
            .map_err(|e: io::Error| crate::error::ExitError::Other(e.into()))?
        {
            if let Event::Key(key) =
                event::read().map_err(|e: io::Error| crate::error::ExitError::Other(e.into()))?
            {
                if key.kind == KeyEventKind::Press
                    && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                {
                    drop(guard);
                    return Ok(());
                }
            }
        }
    }
}

fn draw(frame: &mut Frame, state: &AppState) {
    let theme = state.theme;
    let area = frame.area();
    frame.render_widget(
        ratatui::widgets::Block::default().style(Style::default().bg(theme.ground).fg(theme.ink)),
        area,
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    draw_masthead(frame, chunks[0], state);
    draw_body(frame, chunks[1], state);
    draw_cli_line(frame, chunks[2], state);
    draw_footer(frame, chunks[3], state);
}

fn draw_masthead(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.theme;
    let left_text: &str = if state.recording {
        "REC"
    } else {
        state.tool_name.as_str()
    };
    let mut left_spans = vec![];
    if state.recording {
        left_spans.push(Span::styled(
            "●",
            Style::default()
                .fg(theme.failed)
                .add_modifier(Modifier::BOLD),
        ));
        left_spans.push(Span::raw(" "));
    }
    left_spans.push(Span::styled(
        left_text,
        Style::default().add_modifier(Modifier::BOLD),
    ));

    let right = state.masthead_meta();
    let left_len: usize = left_spans.iter().map(|s| s.content.chars().count()).sum();
    let gap = (area.width as usize).saturating_sub(left_len + right.chars().count() + 2);

    let mut spans = left_spans;
    spans.push(Span::raw(" ".repeat(gap.max(1))));
    spans.push(Span::styled(right, Style::default().fg(theme.dim)));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_cli_line(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.theme;
    let cmd = format!(
        "curl -N http://{}:{}/api/record/stream",
        state.host, state.port
    );
    let line = Line::from(vec![
        Span::styled("$ ", Style::default().fg(theme.accent)),
        Span::styled(cmd, Style::default().fg(theme.dim)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_footer(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.theme;
    let keys = "q quit";
    let left_len = state.log.chars().count();
    let gap = (area.width as usize).saturating_sub(left_len + keys.len() + 2);
    let line = Line::from(vec![
        Span::styled(
            state.log.clone(),
            Style::default()
                .fg(theme.ink)
                .add_modifier(Modifier::ITALIC),
        ),
        Span::raw(" ".repeat(gap.max(1))),
        Span::styled(keys, Style::default().fg(theme.dim)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_body(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.theme;
    let mut lines: Vec<Line> = Vec::new();

    if state.connection == Connection::Unreachable {
        lines.push(Line::styled(
            format!("cannot reach {}:{}", state.host, state.port),
            Style::default()
                .fg(theme.failed)
                .add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "is `lrex serve` running? this TUI is a client of its SSE stream,",
            Style::default().fg(theme.dim),
        ));
        lines.push(Line::styled(
            "not a second capture pipeline.",
            Style::default().fg(theme.dim),
        ));
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    lines.push(Line::styled(
        format_timecode(state.elapsed_s),
        Style::default().add_modifier(Modifier::BOLD),
    ));
    lines.push(Line::raw(""));

    if !state.recording {
        lines.push(Line::styled(
            "waiting for a recording to start…",
            Style::default().fg(theme.dim),
        ));
        lines.push(Line::styled(
            "(start one with `lrex record ...`, or from the web UI)",
            Style::default().fg(theme.dim2),
        ));
    } else {
        for lvl in &state.levels {
            lines.push(meter_line(&lvl.name, lvl.peak_dbfs, lvl.rms_dbfs, &theme));
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!("frames {} · xruns {}", state.frames, state.xruns),
            Style::default().fg(if state.xruns > 0 {
                theme.failed
            } else {
                theme.dim
            }),
        ));
        if !state.active_midi.is_empty() || state.midi_events > 0 {
            lines.push(Line::styled(
                format!(
                    "midi: {} held · {} events",
                    state.active_midi.len(),
                    state.midi_events
                ),
                Style::default().fg(theme.dim),
            ));
        }
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// A cliamp-style level meter: a fixed-width filled bar plus the numeric
/// dBFS readout. `-60..0 dBFS` maps to `0..100%` of the bar. LUFS/Catppuccin
/// color the last ~15%/4% as warning/hot zones, matching a classic VU meter;
/// Mono uses a single flat fill with no hue at all (state is the number,
/// never the color, per the locked style study).
fn meter_line(name: &str, peak_dbfs: f64, rms_dbfs: f64, theme: &Theme) -> Line<'static> {
    const WIDTH: usize = 28;
    let ratio = ((peak_dbfs + 60.0) / 60.0).clamp(0.0, 1.0);
    let filled = (ratio * WIDTH as f64).round() as usize;

    let fill_style = if theme.is_mono() {
        Style::default().fg(theme.ink)
    } else if ratio > 0.92 {
        Style::default().fg(theme.failed)
    } else if ratio > 0.75 {
        Style::default().fg(theme.running)
    } else {
        Style::default().fg(theme.done)
    };

    let bar: String = "█".repeat(filled) + &"░".repeat(WIDTH - filled);
    Line::from(vec![
        Span::styled(format!("{name:<6}"), Style::default().fg(theme.dim)),
        Span::styled(bar, fill_style),
        Span::styled(
            format!(" {peak_dbfs:6.1} pk / {rms_dbfs:6.1} rms dBFS"),
            Style::default().fg(theme.dim),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecode_starts_at_one_hour() {
        assert_eq!(format_timecode(0.0), "01:00:00.00");
    }

    #[test]
    fn timecode_advances_correctly() {
        assert_eq!(format_timecode(3.5), "01:00:03.50");
        assert_eq!(format_timecode(3661.0), "02:01:01.00");
    }

    #[test]
    fn timecode_never_goes_negative() {
        // Defensive: even a bogus negative elapsed_s clamps to the base hour
        // rather than underflowing.
        assert_eq!(format_timecode(-5.0), "01:00:00.00");
    }

    /// No real audio hardware exists in this sandbox to drive an actual
    /// recording through `serve`, so the "recording" rendering path can't be
    /// exercised live the way the idle/unreachable paths were (see the PTY
    /// verification referenced in the PR description). This is the
    /// schema-accurate substitute: a literal fixture matching the REAL
    /// `event: "progress"` shape emitted by `record::run` (confirmed by
    /// reading the actual `serde_json::json!({...})` call site, not
    /// guessed), fed through the exact same `apply_frame` the live client
    /// uses, asserting the parsed state is what the renderer would draw.
    fn real_shaped_recording_frame() -> serde_json::Value {
        serde_json::json!({
            "recording": true,
            "elapsed_s": 47.25,
            "name": "take-0042",
            "frames": 2_268_000u64,
            "xruns": 0,
            "levels": [
                {"name": "mic", "peak_dbfs": -6.2, "rms_dbfs": -14.8, "wave": [0.1, -0.2, 0.05]},
                {"name": "piano", "peak_dbfs": -18.4, "rms_dbfs": -26.1, "wave": []}
            ],
            "active": [60, 64],
            "midi_events": 12
        })
    }

    #[test]
    fn apply_frame_parses_the_real_progress_shape() {
        let mut state = AppState::new("127.0.0.1".to_string(), 8777, Theme::lufs());
        state.apply_frame(real_shaped_recording_frame());

        assert!(state.recording);
        assert_eq!(state.take_name.as_deref(), Some("take-0042"));
        assert!((state.elapsed_s - 47.25).abs() < 1e-9);
        assert_eq!(state.frames, 2_268_000);
        assert_eq!(state.xruns, 0);
        assert_eq!(state.levels.len(), 2);
        assert_eq!(state.levels[0].name, "mic");
        assert!((state.levels[0].peak_dbfs - -6.2).abs() < 1e-9);
        assert!((state.levels[0].rms_dbfs - -14.8).abs() < 1e-9);
        assert_eq!(state.levels[1].name, "piano");
        assert_eq!(state.active_midi, vec![60, 64]);
        assert_eq!(state.midi_events, 12);
    }

    #[test]
    fn apply_frame_idle_clears_recording_state() {
        let mut state = AppState::new("127.0.0.1".to_string(), 8777, Theme::lufs());
        state.apply_frame(real_shaped_recording_frame());
        assert!(state.recording);

        // The exact idle frame `stream_record` sends -- confirmed both by
        // reading the source and by curling a real `serve` process.
        state.apply_frame(serde_json::json!({"recording": false}));

        assert!(!state.recording);
        assert!(state.take_name.is_none());
        assert!(state.levels.is_empty());
        assert_eq!(state.elapsed_s, 0.0);
    }

    #[test]
    fn apply_frame_missing_optional_midi_fields_does_not_panic() {
        // Most frames have no MIDI armed at all -- `notes`/`active`/
        // `midi_events` are entirely absent from the JSON, not present-but-null.
        let mut state = AppState::new("127.0.0.1".to_string(), 8777, Theme::lufs());
        state.apply_frame(serde_json::json!({
            "recording": true,
            "elapsed_s": 1.0,
            "name": "no-midi-take",
            "frames": 48000u64,
            "xruns": 0,
            "levels": [{"name": "mic", "peak_dbfs": -10.0, "rms_dbfs": -20.0, "wave": []}]
        }));
        assert!(state.recording);
        assert!(state.active_midi.is_empty());
        assert_eq!(state.midi_events, 0);
    }

    #[test]
    fn masthead_meta_reports_unreachable_distinctly_from_idle() {
        let mut state = AppState::new("127.0.0.1".to_string(), 8777, Theme::lufs());
        assert!(state.masthead_meta().contains("idle"));
        state.connection = Connection::Unreachable;
        assert!(state.masthead_meta().contains("unreachable"));
    }

    #[test]
    fn tool_name_matches_how_the_rest_of_the_binary_names_itself() {
        // Not a specific literal (argv[0] under `cargo test` is the test
        // binary, not lrex/lufs-recorder) -- the point is that the TUI
        // consults the same `crate::invoked_name()` every other surface
        // (help text, error messages) uses, instead of hardcoding one alias.
        let state = AppState::new("127.0.0.1".to_string(), 8777, Theme::lufs());
        assert_eq!(state.tool_name, crate::invoked_name());
    }

    #[test]
    fn is_stale_false_before_any_time_has_meaningfully_passed() {
        let state = AppState::new("127.0.0.1".to_string(), 8777, Theme::lufs());
        // Fresh state defaults to `Connecting`, not `Connected` -- staleness
        // is only a meaningful concept once a connection actually exists.
        assert!(!state.is_stale());
    }

    #[test]
    fn is_stale_true_when_connected_but_quiet_past_the_threshold() {
        let mut state = AppState::new("127.0.0.1".to_string(), 8777, Theme::lufs());
        state.connection = Connection::Connected;
        state.last_event_at = Instant::now() - Duration::from_secs(5);
        assert!(state.is_stale());
        assert!(state.masthead_meta().contains("stale"));
    }

    #[test]
    fn is_stale_false_right_after_a_frame_refreshes_last_event_at() {
        let mut state = AppState::new("127.0.0.1".to_string(), 8777, Theme::lufs());
        state.connection = Connection::Connected;
        state.last_event_at = Instant::now() - Duration::from_secs(5);
        assert!(state.is_stale());
        state.apply_frame(serde_json::json!({"recording": false}));
        assert!(!state.is_stale(), "a fresh frame must clear staleness");
    }

    #[test]
    fn is_stale_never_flagged_while_unreachable() {
        // Unreachable already has its own, more specific message -- stale
        // would be redundant noise stacked on top of it.
        let mut state = AppState::new("127.0.0.1".to_string(), 8777, Theme::lufs());
        state.connection = Connection::Unreachable;
        state.last_event_at = Instant::now() - Duration::from_secs(30);
        assert!(!state.is_stale());
        assert!(!state.masthead_meta().contains("stale"));
    }
}
