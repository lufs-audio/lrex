//! Command-line surface (clap) and the channel-spec parser.
//!
//! Strong defaults preserve the one-button ethos of the original maxpatch; every
//! knob is optional and, where sensible, falls back to the config file.

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// Universal, agent-first audio + MIDI recorder. Ships as two identical
/// binaries — `lufs-recorder` (full name) and `lrex` (short, bplate-compliant
/// alias) — see `lufs_recorder::run_cli()`, which overrides this derived
/// command's displayed name/bin_name at runtime to match however it was
/// actually invoked. The `name` here is intentionally omitted (falls back to
/// `CARGO_PKG_NAME`) since it's never actually shown — `run_cli()` always sets
/// it explicitly before parsing.
///
/// Point it at any audio interface, capture an arbitrary subset of that device's
/// channels alongside MIDI, and get back a take that is *proven* correct.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Emit machine-readable JSON (the agent entry point).
    #[arg(long, global = true)]
    pub json: bool,

    /// Path to a config file. Overrides $LUFS_RECORDER_CONFIG and the default
    /// (~/.config/lufs-recorder/config.toml).
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List audio input devices + channels and MIDI input ports.
    Devices,

    /// Capture audio (+ optional MIDI) to a verified take.
    Record(RecordArgs),

    /// Re-run the verification contract against an existing take directory.
    Verify {
        /// Path to an existing take directory.
        take_dir: PathBuf,
    },

    /// Run in-process self-tests (audio de-interleave + WAV round-trip, MIDI
    /// SMF export, A/V anchor math). No audio hardware required.
    Selftest,

    /// Serve the control UI + JSON API over HTTP (the browser front end).
    Serve {
        /// Address to bind. Default 127.0.0.1 (local only); use 0.0.0.0 to
        /// expose the UI on your LAN.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to bind.
        #[arg(long, default_value_t = 8777)]
        port: u16,

        /// Serve the UI from this directory (its index.html) instead of the
        /// page embedded in the binary — for live frontend iteration.
        #[arg(long, value_name = "DIR")]
        frontend: Option<PathBuf>,
    },

    /// Write a commented default config (maxpatch parity) so you can edit the
    /// device, MIDI port, and output folder for this machine.
    InitConfig {
        /// Where to write it (default: ~/.config/lufs-recorder/config.toml).
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,

        /// Overwrite an existing config file.
        #[arg(long)]
        force: bool,
    },
}

/// `record` options. Anything omitted falls back to the config file, then to
/// the maxpatch-parity defaults.
#[derive(Args, Debug, Default)]
pub struct RecordArgs {
    /// Audio input device (name or substring). Default: config / system default.
    /// Single-device only — unchanged from v0.2. For multi-device capture, use
    /// `--device-track` instead (it's the sole multi-device surface, so there is
    /// never ambiguity about which tracks belong to which device).
    #[arg(long, conflicts_with = "device_track")]
    pub device: Option<String>,

    /// Channel layout, e.g. "1-2,9-10" (two stereo tracks) or "1,2" (two mono
    /// tracks). A range "a-b" is one grouped track; a bare "n" is its own track.
    /// Overrides the config's track layout. Single-device only; use
    /// `--device-track` for multi-device.
    #[arg(long)]
    pub channels: Option<String>,

    /// Define a named track (repeatable): NAME=CHANNELS, e.g.
    /// `--track mic=1,2 --track piano=9-10 --track room=3-6`. Everything after
    /// '=' goes into ONE file (commas and ranges both expand). Overrides the
    /// config track layout; can't be combined with --channels. Applies to the
    /// first (or only) `--device`; use `--device-track` for explicit per-device
    /// assignment across multiple devices.
    #[arg(
        long = "track",
        value_name = "NAME=CHANNELS",
        conflicts_with = "channels"
    )]
    pub track: Vec<String>,

    /// Explicit per-device track assignment for multi-device capture
    /// (repeatable): `DEVICE:NAME=CHANNELS`, e.g.
    /// `--device-track "BlackHole 2ch:mic=1,2" --device-track "BlackHole 16ch:call=1,2"`.
    /// When given, this is the sole source of device+track layout — it can't be
    /// combined with `--device`, `--track`, or `--channels` (ambiguous which
    /// device a bare track belongs to).
    #[arg(
        long = "device-track",
        value_name = "DEVICE:NAME=CHANNELS",
        conflicts_with_all = ["device", "track", "channels"]
    )]
    pub device_track: Vec<String>,

    /// MIDI input to arm: a port name/substring, "all", or "off".
    #[arg(long)]
    pub midi: Option<String>,

    /// Sample rate in Hz. Default: config / device default. All devices in a
    /// multi-device take must resolve to this same rate (no cross-device
    /// resampling) — see CONTRACT.md.
    #[arg(long)]
    pub rate: Option<u32>,

    /// Sample format: 16, 24, or 32f.
    #[arg(long)]
    pub bit_depth: Option<String>,

    /// Output root directory. Default: config out_dir.
    #[arg(long)]
    pub out: Option<PathBuf>,

    /// Optional take label; folder becomes <timestamp>_<label>.
    #[arg(long)]
    pub name: Option<String>,

    /// Fixed capture length in seconds. Omit to run until interrupted (Ctrl-C).
    /// Mutually exclusive with --profile (which computes this from config).
    #[arg(long, conflicts_with = "profile")]
    pub duration: Option<f64>,

    /// Named auto-stop profile from config (e.g. "therapy", "standup"). Sets
    /// the capture duration from the profile's `auto_stop_minutes` +
    /// `buffer_minutes` instead of an explicit --duration.
    #[arg(long)]
    pub profile: Option<String>,

    /// Validate + predict the take without capturing anything.
    #[arg(long)]
    pub dry_run: bool,
}

/// Parse a channel spec into grouped, 1-based channel lists.
///
/// - `"1-2,9-10"` -> `[[1,2],[9,10]]` (two grouped/stereo tracks)
/// - `"1,2"`      -> `[[1],[2]]`      (two mono tracks)
/// - `"1-4"`      -> `[[1,2,3,4]]`    (one 4-channel track)
pub fn parse_channel_groups(spec: &str) -> Result<Vec<Vec<u16>>> {
    let mut groups: Vec<Vec<u16>> = Vec::new();
    for tok in spec.split(',') {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        if let Some((a, b)) = tok.split_once('-') {
            let a: u16 = a
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid channel '{a}' in spec '{spec}'"))?;
            let b: u16 = b
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid channel '{b}' in spec '{spec}'"))?;
            if a == 0 || b == 0 {
                bail!("channels are 1-based; got 0 in spec '{spec}'");
            }
            if b < a {
                bail!("descending range '{tok}' in spec '{spec}'");
            }
            groups.push((a..=b).collect());
        } else {
            let n: u16 = tok
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid channel '{tok}' in spec '{spec}'"))?;
            if n == 0 {
                bail!("channels are 1-based; got 0 in spec '{spec}'");
            }
            groups.push(vec![n]);
        }
    }
    if groups.is_empty() {
        bail!("empty channel spec '{spec}'");
    }
    Ok(groups)
}

/// Parse a flat channel list for a single track: commas and ranges both expand
/// into ONE track's channel list. `"1,2"` -> `[1,2]`; `"1-8"` -> `[1..=8]`;
/// `"1-2,9-10"` -> `[1,2,9,10]`.
pub fn parse_channel_list(spec: &str) -> Result<Vec<u16>> {
    let mut out: Vec<u16> = Vec::new();
    for tok in spec.split(',') {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        if let Some((a, b)) = tok.split_once('-') {
            let a: u16 = a
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid channel '{a}' in spec '{spec}'"))?;
            let b: u16 = b
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid channel '{b}' in spec '{spec}'"))?;
            if a == 0 || b == 0 {
                bail!("channels are 1-based; got 0 in spec '{spec}'");
            }
            if b < a {
                bail!("descending range '{tok}' in spec '{spec}'");
            }
            out.extend(a..=b);
        } else {
            let n: u16 = tok
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid channel '{tok}' in spec '{spec}'"))?;
            if n == 0 {
                bail!("channels are 1-based; got 0 in spec '{spec}'");
            }
            out.push(n);
        }
    }
    if out.is_empty() {
        bail!("empty channel spec '{spec}'");
    }
    Ok(out)
}

/// Parse a `--track NAME=CHANNELS` value into `(name, channels)`.
pub fn parse_track_arg(s: &str) -> Result<(String, Vec<u16>)> {
    let (name, chans) = s
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("--track expects NAME=CHANNELS, got '{s}'"))?;
    let name = name.trim();
    if name.is_empty() {
        bail!("--track name is empty in '{s}'");
    }
    Ok((name.to_string(), parse_channel_list(chans)?))
}

/// Parse a `--device-track DEVICE:NAME=CHANNELS` value into
/// `(device_query, name, channels)`. The device portion is everything before
/// the FIRST `:` (device names don't contain `:` in practice on any backend
/// cpal targets); the remainder is parsed exactly like `--track`.
pub fn parse_device_track_arg(s: &str) -> Result<(String, String, Vec<u16>)> {
    let (device, rest) = s
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("--device-track expects DEVICE:NAME=CHANNELS, got '{s}'"))?;
    let device = device.trim();
    if device.is_empty() {
        bail!("--device-track device is empty in '{s}'");
    }
    let (name, channels) = parse_track_arg(rest)?;
    Ok((device.to_string(), name, channels))
}

/// A default, human-friendly track name for a CLI-specified channel group.
pub fn default_track_name(channels: &[u16]) -> String {
    match channels {
        [] => "track".to_string(),
        [only] => format!("track-{only:02}"),
        [first, .., last] => format!("track-{first:02}-{last:02}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stereo_pairs() {
        assert_eq!(
            parse_channel_groups("1-2,9-10").unwrap(),
            vec![vec![1, 2], vec![9, 10]]
        );
    }

    #[test]
    fn parses_mono_list() {
        assert_eq!(parse_channel_groups("1,2").unwrap(), vec![vec![1], vec![2]]);
    }

    #[test]
    fn parses_single_wide_group() {
        assert_eq!(parse_channel_groups("1-4").unwrap(), vec![vec![1, 2, 3, 4]]);
    }

    #[test]
    fn rejects_zero_and_descending_and_empty() {
        assert!(parse_channel_groups("0").is_err());
        assert!(parse_channel_groups("4-2").is_err());
        assert!(parse_channel_groups("").is_err());
        assert!(parse_channel_groups("x").is_err());
    }

    #[test]
    fn names_are_sensible() {
        assert_eq!(default_track_name(&[1]), "track-01");
        assert_eq!(default_track_name(&[9, 10]), "track-09-10");
    }

    #[test]
    fn channel_list_is_flat() {
        assert_eq!(parse_channel_list("1,2").unwrap(), vec![1, 2]);
        assert_eq!(parse_channel_list("1-4").unwrap(), vec![1, 2, 3, 4]);
        assert_eq!(parse_channel_list("1-2,9-10").unwrap(), vec![1, 2, 9, 10]);
        assert!(parse_channel_list("0").is_err());
        assert!(parse_channel_list("").is_err());
    }

    #[test]
    fn track_arg_parses_name_and_channels() {
        assert_eq!(
            parse_track_arg("piano=9-10").unwrap(),
            ("piano".to_string(), vec![9, 10])
        );
        assert_eq!(
            parse_track_arg("drums=1-8").unwrap(),
            ("drums".to_string(), vec![1, 2, 3, 4, 5, 6, 7, 8])
        );
        assert!(parse_track_arg("noequals").is_err());
        assert!(parse_track_arg("=1,2").is_err());
    }

    #[test]
    fn device_track_arg_parses_device_name_and_channels() {
        assert_eq!(
            parse_device_track_arg("BlackHole 2ch:mic=1,2").unwrap(),
            ("BlackHole 2ch".to_string(), "mic".to_string(), vec![1, 2])
        );
        assert_eq!(
            parse_device_track_arg("BlackHole 16ch:call=1-2").unwrap(),
            ("BlackHole 16ch".to_string(), "call".to_string(), vec![1, 2])
        );
        assert!(parse_device_track_arg("no-colon-here").is_err());
        assert!(parse_device_track_arg(":mic=1,2").is_err());
        assert!(parse_device_track_arg("Device:noequals").is_err());
    }
}
