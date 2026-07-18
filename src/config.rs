//! Config file: defaults reproduce the midi-audio-recorder maxpatch rig; the
//! file lets you point the tool at whatever *this* machine is connected to and
//! choose where takes are written.
//!
//! Resolution order for the config path:
//!   1. `--config <FILE>`
//!   2. `$LUFS_RECORDER_CONFIG`
//!   3. `~/.config/lufs-recorder/config.toml`
//!
//! If none exists, the built-in maxpatch-parity defaults are used.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The maxpatch default save path was
/// `~/Samples/sampleLibrary/midi-audio-recorder_max/`; we keep the family but
/// name the new tool.
const DEFAULT_OUT_DIR: &str = "~/Samples/sampleLibrary/lufs-recorder";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Audio input device name/substring, or "default" for the system default.
    pub device: String,
    /// Sample rate: "default" (device default) or a number like "48000".
    pub rate: String,
    /// Bit depth for written WAVs: "16" | "24" | "32f". Maxpatch = "24".
    pub bit_depth: String,
    /// Where timestamped take folders are written ("~" is expanded).
    pub out_dir: String,
    /// MIDI input port name/substring, "all", or "off". Maxpatch = "Nord Stage 3".
    pub midi_port: String,
    /// Audio track layout. Each entry becomes one WAV file capturing the listed
    /// 1-based device input channels.
    pub tracks: Vec<TrackConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackConfig {
    pub name: String,
    pub channels: Vec<u16>,
}

impl Default for Config {
    /// Maxpatch parity: mic on 1-2, piano on 9-10, 24-bit, Nord Stage 3.
    fn default() -> Self {
        Config {
            device: "default".to_string(),
            rate: "default".to_string(),
            bit_depth: "24".to_string(),
            out_dir: DEFAULT_OUT_DIR.to_string(),
            midi_port: "Nord Stage 3".to_string(),
            tracks: vec![
                TrackConfig {
                    name: "mic".to_string(),
                    channels: vec![1, 2],
                },
                TrackConfig {
                    name: "piano".to_string(),
                    channels: vec![9, 10],
                },
            ],
        }
    }
}

impl Config {
    /// Load the config from an explicit path, the env var, or the default
    /// location. Returns the parity defaults if nothing is found.
    pub fn load(explicit: Option<&Path>) -> Result<(Config, Option<PathBuf>)> {
        if let Some(path) = resolve_path(explicit) {
            if path.exists() {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading config {}", path.display()))?;
                let cfg: Config = toml::from_str(&text)
                    .with_context(|| format!("parsing config {}", path.display()))?;
                cfg.validate()?;
                return Ok((cfg, Some(path)));
            }
        }
        Ok((Config::default(), None))
    }

    fn validate(&self) -> Result<()> {
        validate_bit_depth(&self.bit_depth)?;
        if self.rate != "default" {
            self.rate.parse::<u32>().with_context(|| {
                format!("rate must be \"default\" or a number, got {:?}", self.rate)
            })?;
        }
        if self.tracks.is_empty() {
            anyhow::bail!("config has no [[tracks]]; at least one is required");
        }
        for t in &self.tracks {
            if t.channels.is_empty() {
                anyhow::bail!("track {:?} has no channels", t.name);
            }
            if t.channels.contains(&0) {
                anyhow::bail!("track {:?} has a 0 channel; channels are 1-based", t.name);
            }
        }
        Ok(())
    }

    /// Parsed rate, or `None` for "device default".
    pub fn rate_hz(&self) -> Option<u32> {
        if self.rate == "default" {
            None
        } else {
            self.rate.parse().ok()
        }
    }

    /// Absolute output directory with "~" expanded.
    pub fn out_path(&self) -> PathBuf {
        expand_tilde(&self.out_dir)
    }
}

pub fn validate_bit_depth(s: &str) -> Result<()> {
    match s {
        "16" | "24" | "32f" => Ok(()),
        other => anyhow::bail!("bit_depth must be 16, 24, or 32f, got {other:?}"),
    }
}

/// The default config file location: ~/.config/lufs-recorder/config.toml.
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("lufs-recorder").join("config.toml"))
}

fn resolve_path(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p.to_path_buf());
    }
    if let Ok(env) = std::env::var("LUFS_RECORDER_CONFIG") {
        if !env.is_empty() {
            return Some(PathBuf::from(env));
        }
    }
    default_config_path()
}

/// Expand a leading "~" to the user's home directory.
pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if p == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(p)
}

/// A commented default config, ready to edit. Written by `init-config`.
pub fn default_config_toml() -> String {
    format!(
        r#"# lufs-recorder config
# Defaults below reproduce the midi-audio-recorder Max/MSP patch:
#   mic on channels 1-2, piano on 9-10, 24-bit, MIDI from a Nord Stage 3.
# Run `lufs-recorder devices --json` to see the exact device/port names on THIS
# machine, then edit `device` and `midi_port` to match.

# Audio input device name or substring; "default" = system default input.
device = "default"

# Sample rate: "default" (the device's current rate) or a number, e.g. "48000".
rate = "default"

# Bit depth for written WAVs: "16" | "24" | "32f". Maxpatch = 24.
bit_depth = "24"

# Where timestamped take folders are written ("~" is expanded to your home dir).
out_dir = "{DEFAULT_OUT_DIR}"

# MIDI input to arm: a port name/substring, "all", or "off". Maxpatch = Nord Stage 3.
midi_port = "Nord Stage 3"

# Audio track layout. Each [[tracks]] becomes one WAV file capturing the listed
# 1-based device input channels (two channels = a stereo file).
[[tracks]]
name = "mic"
channels = [1, 2]

[[tracks]]
name = "piano"
channels = [9, 10]
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_maxpatch_parity() {
        let c = Config::default();
        assert_eq!(c.bit_depth, "24");
        assert_eq!(c.midi_port, "Nord Stage 3");
        assert_eq!(c.tracks.len(), 2);
        assert_eq!(c.tracks[0].channels, vec![1, 2]);
        assert_eq!(c.tracks[1].channels, vec![9, 10]);
    }

    #[test]
    fn emitted_default_config_roundtrips_and_validates() {
        let toml_text = default_config_toml();
        let cfg: Config = toml::from_str(&toml_text).expect("default config must parse");
        cfg.validate().expect("default config must validate");
        assert_eq!(cfg.tracks.len(), 2);
    }

    #[test]
    fn rejects_bad_bit_depth() {
        assert!(validate_bit_depth("20").is_err());
        assert!(validate_bit_depth("24").is_ok());
        assert!(validate_bit_depth("32f").is_ok());
    }

    #[test]
    fn rate_hz_parses() {
        let mut c = Config::default();
        assert_eq!(c.rate_hz(), None);
        c.rate = "48000".into();
        assert_eq!(c.rate_hz(), Some(48000));
    }
}
