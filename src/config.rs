//! Config file: defaults reproduce the midi-audio-recorder maxpatch rig; the
//! file lets you point the tool at whatever *this* machine is connected to and
//! choose where takes are written.
//!
//! Resolution order for the config path:
//!   1. `--config <FILE>`
//!   2. `$LUFS_RECORDER_CONFIG`
//!   3. `~/.config/lufs-recorder/config.toml`
//!
//! If none exists, the built-in maxpatch-parity defaults are used. On top of
//! whichever of those is loaded, an optional project-local `.lufs-recorder.toml`
//! in the current working directory is layered over it — see
//! [`ProjectOverride`] and `load()` below.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The maxpatch default save path was
/// `~/Samples/sampleLibrary/midi-audio-recorder_max/`; we keep the family but
/// name the new tool.
const DEFAULT_OUT_DIR: &str = "~/Samples/sampleLibrary/lufs-recorder";

/// Filename for a project-scoped config override, discovered relative to the
/// current working directory (see `load()`).
const PROJECT_OVERRIDE_FILENAME: &str = ".lufs-recorder.toml";

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
    /// Named auto-stop profiles for `record --profile <name>`, e.g. a "therapy"
    /// profile that auto-stops a call recording after an hour plus a buffer.
    /// Entirely user-defined — nothing here is hardcoded into the binary.
    /// Absent from a config file, this defaults to empty so every pre-existing
    /// config keeps loading and behaving exactly as before.
    #[serde(default)]
    pub profiles: HashMap<String, ProfileConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackConfig {
    pub name: String,
    pub channels: Vec<u16>,
}

/// A named auto-stop profile: `record --profile therapy` sets the capture
/// duration from these two numbers instead of requiring an explicit
/// `--duration`. See [`resolve_profile_duration_secs`] for the exact math.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    /// Minutes from recording start after which capture auto-stops.
    pub auto_stop_minutes: f64,
    /// Minutes of slack applied on both ends (see `resolve_profile_duration_secs`
    /// doc comment for the exact interpretation and why).
    pub buffer_minutes: f64,
}

/// The project-local override file (`.lufs-recorder.toml`): every field is
/// optional, and only the fields actually present override the base config —
/// this is NOT a full `Config`, so a project file only needs to name what it
/// wants to change (e.g. just `device`, for a machine-specific interface).
/// `deny_unknown_fields` still applies: a typo'd field name fails loudly rather
/// than being silently ignored, exactly like the base config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectOverride {
    device: Option<String>,
    rate: Option<String>,
    bit_depth: Option<String>,
    out_dir: Option<String>,
    midi_port: Option<String>,
    tracks: Option<Vec<TrackConfig>>,
    /// Profiles merge key-by-key into the base config's profile map (a project
    /// file can add or override individual named profiles without repeating
    /// every profile from the global config).
    profiles: Option<HashMap<String, ProfileConfig>>,
}

impl ProjectOverride {
    /// Apply this override onto `base` in place — only `Some` fields change.
    fn apply_to(self, base: &mut Config) {
        if let Some(v) = self.device {
            base.device = v;
        }
        if let Some(v) = self.rate {
            base.rate = v;
        }
        if let Some(v) = self.bit_depth {
            base.bit_depth = v;
        }
        if let Some(v) = self.out_dir {
            base.out_dir = v;
        }
        if let Some(v) = self.midi_port {
            base.midi_port = v;
        }
        if let Some(v) = self.tracks {
            base.tracks = v;
        }
        if let Some(v) = self.profiles {
            base.profiles.extend(v);
        }
    }
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
            profiles: HashMap::new(),
        }
    }
}

impl Config {
    /// Load the config from an explicit path, the env var, or the default
    /// location (returns the parity defaults if nothing is found), THEN layer
    /// an optional `.lufs-recorder.toml` from the current working directory on
    /// top of it. Project-override discovery failures (missing file) are silent
    /// — it's opt-in; parse/validation failures in a file that DOES exist are
    /// not (same `deny_unknown_fields` strictness as the base config, so a
    /// typo'd field in the project file fails loudly rather than being ignored).
    pub fn load(explicit: Option<&Path>) -> Result<(Config, Option<PathBuf>)> {
        let (mut cfg, source) = if let Some(path) = resolve_path(explicit) {
            if path.exists() {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading config {}", path.display()))?;
                let cfg: Config = toml::from_str(&text)
                    .with_context(|| format!("parsing config {}", path.display()))?;
                cfg.validate()?;
                (cfg, Some(path))
            } else {
                (Config::default(), None)
            }
        } else {
            (Config::default(), None)
        };

        if let Some(project_path) = project_override_path() {
            if project_path.exists() {
                let text = std::fs::read_to_string(&project_path).with_context(|| {
                    format!("reading project config {}", project_path.display())
                })?;
                let over: ProjectOverride = toml::from_str(&text).with_context(|| {
                    format!("parsing project config {}", project_path.display())
                })?;
                over.apply_to(&mut cfg);
                cfg.validate().with_context(|| {
                    format!(
                        "config invalid after applying project override {}",
                        project_path.display()
                    )
                })?;
            }
        }

        Ok((cfg, source))
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
        for (name, p) in &self.profiles {
            if p.auto_stop_minutes <= 0.0 {
                anyhow::bail!("profile {name:?} auto_stop_minutes must be > 0");
            }
            if p.buffer_minutes < 0.0 {
                anyhow::bail!("profile {name:?} buffer_minutes must be >= 0");
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

/// Resolve `record --profile <name>` against `cfg.profiles` into a capture
/// duration in seconds.
///
/// Interpretation of "buffer applies on both ends" (the proposal's own words —
/// confirmed with Daniel 2026-08-21): the buffer is added symmetrically around
/// the auto-stop length, measured from *actual* recording start —
/// `(auto_stop_minutes + 2 * buffer_minutes) * 60`. "Start early" is the
/// caller's responsibility (the agent/automation invokes `record --profile`
/// before the nominal event time); this function only ever needs to know when
/// recording actually started, never a separate "nominal" event time, so no new
/// scheduling concept exists anywhere in the config or API surface.
///
/// Returns an explicit error for an unknown profile name — never a silent
/// fallback to an undocumented default duration.
pub fn resolve_profile_duration_secs(cfg: &Config, profile_name: &str) -> Result<f64> {
    let p = cfg.profiles.get(profile_name).ok_or_else(|| {
        let known: Vec<&str> = cfg.profiles.keys().map(|s| s.as_str()).collect();
        anyhow::anyhow!(
            "unknown profile {profile_name:?}; configured profiles: {}",
            if known.is_empty() {
                "(none)".to_string()
            } else {
                known.join(", ")
            }
        )
    })?;
    Ok((p.auto_stop_minutes + 2.0 * p.buffer_minutes) * 60.0)
}

pub fn validate_bit_depth(s: &str) -> Result<()> {
    match s {
        "16" | "24" | "32f" => Ok(()),
        other => anyhow::bail!("bit_depth must be 16, 24, or 32f, got {other:?}"),
    }
}

/// The default config file location: ~/.config/lufs-recorder/config.toml.
///
/// Standardized on `~/.config` on every platform — shell-friendly (no spaces
/// like macOS's "Application Support") and consistent across machines. The
/// `--config` flag and `$LUFS_RECORDER_CONFIG` still override it.
pub fn default_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".config").join("lufs-recorder").join("config.toml"))
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

/// `.lufs-recorder.toml` in the current working directory, or `None` if the
/// cwd can't be determined (never fatal — project override is opt-in).
fn project_override_path() -> Option<PathBuf> {
    std::env::current_dir()
        .ok()
        .map(|d| d.join(PROJECT_OVERRIDE_FILENAME))
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

# Named auto-stop profiles for `record --profile <name>` (optional; none by
# default). Example — uncomment and adjust for a voice-call automation:
# [profiles.therapy]
# auto_stop_minutes = 60
# buffer_minutes = 10
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
        assert!(c.profiles.is_empty());
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

    #[test]
    fn old_config_without_profiles_table_still_parses() {
        // Exactly today's emitted config, pre-profiles: no [profiles.*] section
        // at all. `#[serde(default)]` must make this load exactly as before.
        let text = r#"
device = "default"
rate = "default"
bit_depth = "24"
out_dir = "~/Samples/sampleLibrary/lufs-recorder"
midi_port = "Nord Stage 3"

[[tracks]]
name = "mic"
channels = [1, 2]
"#;
        let cfg: Config = toml::from_str(text).expect("pre-profiles config must still parse");
        cfg.validate().expect("must still validate");
        assert!(cfg.profiles.is_empty());
    }

    #[test]
    fn profile_duration_is_auto_stop_plus_symmetric_buffer() {
        let mut c = Config::default();
        c.profiles.insert(
            "therapy".to_string(),
            ProfileConfig {
                auto_stop_minutes: 60.0,
                buffer_minutes: 10.0,
            },
        );
        // (60 + 2*10) * 60 = 4800s.
        let secs = resolve_profile_duration_secs(&c, "therapy").unwrap();
        assert!((secs - 4800.0).abs() < 1e-9, "got {secs}");
    }

    #[test]
    fn profile_duration_zero_buffer_is_just_auto_stop() {
        let mut c = Config::default();
        c.profiles.insert(
            "standup".to_string(),
            ProfileConfig {
                auto_stop_minutes: 30.0,
                buffer_minutes: 0.0,
            },
        );
        let secs = resolve_profile_duration_secs(&c, "standup").unwrap();
        assert!((secs - 1800.0).abs() < 1e-9, "got {secs}");
    }

    #[test]
    fn unknown_profile_is_an_explicit_error_not_a_silent_default() {
        let c = Config::default();
        let err = resolve_profile_duration_secs(&c, "nope").unwrap_err();
        assert!(err.to_string().contains("unknown profile"));
    }

    #[test]
    fn validate_rejects_non_positive_auto_stop() {
        let mut c = Config::default();
        c.profiles.insert(
            "bad".to_string(),
            ProfileConfig {
                auto_stop_minutes: 0.0,
                buffer_minutes: 0.0,
            },
        );
        assert!(c.validate().is_err());
    }

    #[test]
    fn project_override_changes_only_named_fields() {
        let mut base = Config::default();
        let over_text = r#"
device = "BlackHole 2ch"

[profiles.therapy]
auto_stop_minutes = 60
buffer_minutes = 10
"#;
        let over: ProjectOverride = toml::from_str(over_text).unwrap();
        over.apply_to(&mut base);
        assert_eq!(base.device, "BlackHole 2ch");
        // Untouched fields keep their base values.
        assert_eq!(base.bit_depth, "24");
        assert_eq!(base.tracks.len(), 2);
        assert_eq!(base.profiles.len(), 1);
        assert!(base.profiles.contains_key("therapy"));
    }

    #[test]
    fn project_override_rejects_unknown_field() {
        let bad = r#"not_a_real_field = "oops""#;
        let parsed: std::result::Result<ProjectOverride, _> = toml::from_str(bad);
        assert!(
            parsed.is_err(),
            "a typo'd project-override field must fail loudly, not be silently ignored"
        );
    }
}
