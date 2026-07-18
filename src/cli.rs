//! Command-line surface (clap) and the channel-spec parser.
//!
//! Strong defaults preserve the one-button ethos of the original maxpatch; every
//! knob is optional and, where sensible, falls back to the config file.

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// Universal, agent-first audio + MIDI recorder.
///
/// Point it at any audio interface, capture an arbitrary subset of that device's
/// channels alongside MIDI, and get back a take that is *proven* correct.
#[derive(Parser, Debug)]
#[command(name = "lufs-recorder", version, about, long_about = None)]
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
    #[arg(long)]
    pub device: Option<String>,

    /// Channel layout, e.g. "1-2,9-10" (two stereo tracks) or "1,2" (two mono
    /// tracks). A range "a-b" is one grouped track; a bare "n" is its own track.
    /// Overrides the config's track layout.
    #[arg(long)]
    pub channels: Option<String>,

    /// MIDI input to arm: a port name/substring, "all", or "off".
    #[arg(long)]
    pub midi: Option<String>,

    /// Sample rate in Hz. Default: config / device default.
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
    #[arg(long)]
    pub duration: Option<f64>,

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
}
