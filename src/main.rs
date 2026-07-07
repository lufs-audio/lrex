//! lufs-recorder — universal, agent-first audio + MIDI recorder.
//!
//! v0.1 is the *honest skeleton*: the full command surface is defined and parses,
//! but no command is implemented yet. Every command therefore FAILS with a clear
//! not-implemented sentinel (exit code 70) rather than pretending to succeed —
//! see the verifiable-correctness doctrine in the design suite
//! (danialrami/agent-knowledge : docs/product/lufs-recorder).
//!
//! Nothing in this binary reports success it has not earned.

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

/// Exit-code taxonomy (see CONTRACT.md). Stable across versions.
mod exit {
    /// Take captured AND verified.
    pub const OK: u8 = 0;
    /// Requested audio device or MIDI port unavailable.
    pub const DEVICE_UNAVAILABLE: u8 = 3;
    /// Requested format (rate / bit-depth / channels) unsupported by the device.
    pub const FORMAT_UNSUPPORTED: u8 = 4;
    /// Capture ran but FAILED the verification contract (xrun, truncation, ...).
    pub const CONTRACT_VIOLATION: u8 = 5;
    /// Interrupted before a valid take was produced.
    pub const INTERRUPTED: u8 = 6;
    /// Not implemented yet — the honest-failure sentinel.
    pub const NOT_IMPLEMENTED: u8 = 70;
}

/// Universal, agent-first audio + MIDI recorder.
///
/// Point it at any audio interface, capture an arbitrary subset of that device's
/// channels alongside MIDI, and get back a take that is *proven* correct.
#[derive(Parser, Debug)]
#[command(name = "lufs-recorder", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Emit machine-readable JSON (the agent entry point).
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List audio devices + channels and MIDI input ports.
    Devices,

    /// Capture audio (+ optional MIDI) to a verified take.
    Record(RecordArgs),

    /// Re-run the verification contract against an existing take.
    Verify {
        /// Path to an existing take directory.
        take_dir: PathBuf,
    },
}

/// `record` options. Strong defaults preserve the one-button ethos of the
/// original maxpatch; every knob is optional.
#[derive(Args, Debug)]
struct RecordArgs {
    /// Audio input device (name or id). Default: system default input.
    #[arg(long)]
    device: Option<String>,

    /// Channel subset, e.g. "1,2" or "1-2,9-10". Default: all channels.
    #[arg(long)]
    channels: Option<String>,

    /// MIDI input to arm: a port name, "all", or "off".
    #[arg(long, default_value = "off")]
    midi: String,

    /// Sample rate in Hz. Default: device default.
    #[arg(long)]
    rate: Option<u32>,

    /// Sample format: 16, 24, or 32f.
    #[arg(long, default_value = "24")]
    bit_depth: String,

    /// Output root directory. Default: ~/Recordings/lufs-recorder.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Optional take label; folder becomes <timestamp>_<label>.
    #[arg(long)]
    name: Option<String>,

    /// Fixed capture length in seconds. Omit to run until interrupted.
    #[arg(long)]
    duration: Option<f64>,

    /// Validate + predict the take without capturing anything.
    #[arg(long)]
    dry_run: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let command_name = match &cli.command {
        Command::Devices => "devices",
        Command::Record(_) => "record",
        Command::Verify { .. } => "verify",
    };

    // v0.1: no command is implemented. Fail honestly.
    not_implemented(command_name, cli.json)
}

/// The honest-failure sentinel. Prints a clear message and returns exit code 70.
/// This lives in the binary (not just a docstring) so a fresh build can never be
/// mistaken for a working recorder.
fn not_implemented(command: &str, json: bool) -> ExitCode {
    let message = format!(
        "`{command}` is not implemented yet (v0.1 skeleton). See CONTRACT.md and the roadmap."
    );

    if json {
        let payload = serde_json::json!({
            "error": "not_implemented",
            "command": command,
            "message": message,
            "exit_code": exit::NOT_IMPLEMENTED,
        });
        println!("{payload}");
    } else {
        eprintln!("lufs-recorder: {message}");
    }

    // Reference the rest of the taxonomy so it isn't dead code before v0.2 wires it in.
    debug_assert_ne!(exit::OK, exit::NOT_IMPLEMENTED);
    let _ = (
        exit::DEVICE_UNAVAILABLE,
        exit::FORMAT_UNSUPPORTED,
        exit::CONTRACT_VIOLATION,
        exit::INTERRUPTED,
    );

    ExitCode::from(exit::NOT_IMPLEMENTED)
}
