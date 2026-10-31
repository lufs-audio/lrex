//! Config file: defaults reproduce the midi-audio-recorder maxpatch rig; the
//! file lets you point the tool at whatever *this* machine is connected to and
//! choose where takes are written.
//!
//! Resolution order for the config path (first-class `lrex` names, with 100%
//! backwards-compatible fallback to every `.lufs-recorder` equivalent):
//!   1. `--config <FILE>`
//!   2. `$LREX_CONFIG`, then `$LUFS_RECORDER_CONFIG`
//!   3. `~/.config/lrex/config.toml` (if it exists on disk)
//!   4. `~/.config/lufs-recorder/config.toml` (if it exists on disk)
//!   5. `~/.config/lrex/config.toml` (the default/creation target)
//!
//! If none exists, the built-in maxpatch-parity defaults are used. On top of
//! whichever of those is loaded, an optional project-local `.lrex.toml` (or,
//! absent that, `.lufs-recorder.toml`) in the current working directory is
//! layered over it — see [`ProjectOverride`] and `load()` below.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The maxpatch default save path was
/// `~/Samples/sampleLibrary/midi-audio-recorder_max/`; we keep the family but
/// name the new tool.
const DEFAULT_OUT_DIR: &str = "~/Samples/sampleLibrary/lufs-recorder";

/// Project-scoped config override filenames, in discovery priority order,
/// relative to the current working directory (see `load()`): `.lrex.toml`
/// first, then the v0.5.1 `.lufs-recorder.toml` for backwards compatibility.
pub const PROJECT_OVERRIDE_FILENAMES: &[&str] = &[".lrex.toml", ".lufs-recorder.toml"];

/// Config directory names under `~/.config`, in priority order: the canonical
/// `lrex` directory first, then the v0.5.1 `lufs-recorder` directory.
pub const CONFIG_DIR_NAMES: &[&str] = &["lrex", "lufs-recorder"];

/// Environment variables naming a config file, in priority order: `LREX_CONFIG`
/// first, then the v0.5.1 `LUFS_RECORDER_CONFIG`.
pub const ENV_CONFIG_VARS: &[&str] = &["LREX_CONFIG", "LUFS_RECORDER_CONFIG"];

/// Subdirectory (under each config dir) holding one named preset per file.
pub const PRESETS_DIR_NAME: &str = "presets";

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
    /// Named presets for `record --preset <name>` / `serve --preset <name>`,
    /// declared inline as `[presets.<name>]` tables. Each preset is a sparse
    /// override of the base config (exactly like a project override). Presets
    /// can also live as standalone files under `~/.config/lrex/presets/` — see
    /// [`resolve_preset`]. Absent from a config file, this defaults to empty so
    /// every pre-existing config keeps loading and behaving exactly as before.
    #[serde(default)]
    pub presets: HashMap<String, PresetConfig>,
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
pub struct ProjectOverride {
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

/// A named preset: a human-readable [`description`](Self::description) plus the
/// sparse set of config fields the preset overrides. Deserializes from either a
/// standalone `~/.config/lrex/presets/<name>.toml` file or an inline
/// `[presets.<name>]` table in the base config — the two share one shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetConfig {
    /// Optional human-readable summary, shown by `presets`/`presets list`.
    #[serde(default)]
    pub description: Option<String>,
    /// The config fields this preset overrides (same sparse semantics as a
    /// project-local override file).
    #[serde(flatten)]
    pub override_fields: ProjectOverride,
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
            presets: HashMap::new(),
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

/// One discoverable preset, as reported by `lrex presets` / `presets list`.
#[derive(Debug, Clone, Serialize)]
pub struct PresetSummary {
    /// The preset name, as passed to `--preset <name>`.
    pub name: String,
    /// Where it came from: the preset file's path, or `"(inline)"` for a
    /// `[presets.<name>]` table in the active base config.
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// List every preset discoverable from `base_cfg`, in deterministic
/// (name-sorted) order. A preset file wins over an inline table of the same
/// name (matching [`resolve_preset`]'s resolution order), and `~/.config/lrex`
/// wins over `~/.config/lufs-recorder`.
pub fn list_available_presets(base_cfg: &Config) -> Vec<PresetSummary> {
    list_available_presets_in(dirs::home_dir().as_deref(), base_cfg)
}

fn list_available_presets_in(home: Option<&Path>, base_cfg: &Config) -> Vec<PresetSummary> {
    let mut out: Vec<PresetSummary> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    if let Some(home) = home {
        for dir in CONFIG_DIR_NAMES {
            let presets_dir = home.join(".config").join(dir).join(PRESETS_DIR_NAME);
            let entries = match std::fs::read_dir(&presets_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                    continue;
                }
                let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if seen.insert(name.to_string()) {
                    out.push(PresetSummary {
                        name: name.to_string(),
                        source: path.display().to_string(),
                        description: read_preset_description(&path),
                    });
                }
            }
        }
    }

    for (name, preset) in &base_cfg.presets {
        if seen.insert(name.clone()) {
            out.push(PresetSummary {
                name: name.clone(),
                source: "(inline)".to_string(),
                description: preset.description.clone(),
            });
        }
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Best-effort read of a preset file's `description` for listing; a malformed
/// preset is not an error *here* (listing must still work) — `resolve_preset`
/// reports it loudly when that preset is actually selected.
fn read_preset_description(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let preset: PresetConfig = toml::from_str(&text).ok()?;
    preset.description
}

/// Resolve `record --preset <name>` / `serve --preset <name>` into a full
/// [`Config`]: the already-loaded `base_cfg` (which itself already includes any
/// project override) with the preset's sparse fields applied on top. CLI flags
/// are applied *after* this by the caller, so they keep top precedence.
///
/// Resolution order for the preset itself:
///   1. file `~/.config/lrex/presets/<name>.toml`
///   2. file `~/.config/lufs-recorder/presets/<name>.toml`
///   3. inline `[presets.<name>]` in `base_cfg`
///
/// An unknown name is an explicit error that lists every discovered preset —
/// never a silent fallback.
pub fn resolve_preset(name: &str, base_cfg: &Config) -> Result<Config> {
    resolve_preset_in(dirs::home_dir().as_deref(), name, base_cfg)
}

/// Testable core of [`resolve_preset`] with the home directory injected, so the
/// file-based resolution order is verifiable without touching `$HOME`.
fn resolve_preset_in(home: Option<&Path>, name: &str, base_cfg: &Config) -> Result<Config> {
    let over = find_preset_override_in(home, name, base_cfg)?.ok_or_else(|| {
        let known: Vec<String> = list_available_presets_in(home, base_cfg)
            .into_iter()
            .map(|p| p.name)
            .collect();
        anyhow::anyhow!(
            "unknown preset {name:?}; available presets: {}",
            if known.is_empty() {
                "(none)".to_string()
            } else {
                known.join(", ")
            }
        )
    })?;
    let mut cfg = base_cfg.clone();
    over.apply_to(&mut cfg);
    cfg.validate()?;
    Ok(cfg)
}

/// Find the sparse override for a preset, honoring the file-then-inline
/// resolution order. A present-but-malformed preset file is a loud error.
fn find_preset_override_in(
    home: Option<&Path>,
    name: &str,
    base_cfg: &Config,
) -> Result<Option<ProjectOverride>> {
    if let Some(home) = home {
        for dir in CONFIG_DIR_NAMES {
            let candidate = home
                .join(".config")
                .join(dir)
                .join(PRESETS_DIR_NAME)
                .join(format!("{name}.toml"));
            if candidate.exists() {
                let text = std::fs::read_to_string(&candidate)
                    .with_context(|| format!("reading preset {}", candidate.display()))?;
                let preset: PresetConfig = toml::from_str(&text)
                    .with_context(|| format!("parsing preset {}", candidate.display()))?;
                return Ok(Some(preset.override_fields));
            }
        }
    }
    Ok(base_cfg
        .presets
        .get(name)
        .map(|p| p.override_fields.clone()))
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
/// The user config path under a specific config directory, e.g.
/// `user_config_path("lrex")` -> `~/.config/lrex/config.toml`.
pub fn user_config_path(dir: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".config").join(dir).join("config.toml"))
}

/// The default (canonical) config file location: `~/.config/lrex/config.toml`.
///
/// Standardized on `~/.config` on every platform — shell-friendly (no spaces
/// like macOS's "Application Support") and consistent across machines. The
/// `--config` flag, `$LREX_CONFIG`, and `$LUFS_RECORDER_CONFIG` still override
/// it, and an existing `~/.config/lufs-recorder/config.toml` is still honored
/// as a fallback (see [`resolve_path`]).
pub fn default_config_path() -> Option<PathBuf> {
    user_config_path(CONFIG_DIR_NAMES[0])
}

/// Resolve which config file to load, in priority order:
///   1. `explicit` (`--config <path>`)
///   2. `$LREX_CONFIG` (if non-empty)
///   3. `$LUFS_RECORDER_CONFIG` (if non-empty)
///   4. `~/.config/lrex/config.toml` (if it exists on disk)
///   5. `~/.config/lufs-recorder/config.toml` (if it exists on disk)
///   6. `~/.config/lrex/config.toml` (the default/creation target)
///
/// Steps 2-6 are delegated to [`resolve_path_with`] with the process's real
/// environment and home directory, so the resolution order is unit-testable
/// without mutating global process state.
pub fn resolve_path(explicit: Option<&Path>) -> Option<PathBuf> {
    resolve_path_with(explicit, |k| std::env::var(k).ok(), dirs::home_dir)
}

/// Testable core of [`resolve_path`]: identical logic with the environment
/// lookup and home-directory lookup injected, so tests never touch (or race on)
/// process-global env vars or `$HOME`.
fn resolve_path_with(
    explicit: Option<&Path>,
    env: impl Fn(&str) -> Option<String>,
    home: impl Fn() -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p.to_path_buf());
    }
    for var in ENV_CONFIG_VARS {
        if let Some(v) = env(var) {
            if !v.is_empty() {
                return Some(PathBuf::from(v));
            }
        }
    }
    // Prefer an already-existing user config, lrex before lufs-recorder.
    for dir in CONFIG_DIR_NAMES {
        if let Some(h) = home() {
            let candidate = h.join(".config").join(dir).join("config.toml");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    // Nothing on disk yet: point at the canonical lrex path (creation target).
    home().map(|h| {
        h.join(".config")
            .join(CONFIG_DIR_NAMES[0])
            .join("config.toml")
    })
}

/// `.lrex.toml` (or, absent that, `.lufs-recorder.toml`) in the current working
/// directory, or `None` if the cwd can't be determined (never fatal — project
/// override is opt-in).
fn project_override_path() -> Option<PathBuf> {
    std::env::current_dir()
        .ok()
        .and_then(|d| project_override_path_in(&d))
}

/// Testable core of [`project_override_path`]: the first of
/// [`PROJECT_OVERRIDE_FILENAMES`] that exists in `dir`, or `None`.
fn project_override_path_in(dir: &Path) -> Option<PathBuf> {
    PROJECT_OVERRIDE_FILENAMES
        .iter()
        .map(|name| dir.join(name))
        .find(|candidate| candidate.exists())
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
        r#"# lrex config (the same file is also read as lufs-recorder's)
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

# Named presets for `record --preset <name>` / `serve --preset <name>`
# (optional; none by default). A preset is a sparse override of the fields
# above, and can also live as its own file: ~/.config/lrex/presets/<name>.toml.
# List them with `presets`. Example — uncomment for desktop/course capture:
# [presets.dante]
# description = "Desktop system audio (Dante course)"
# device = "monitor"
# rate = "48000"
# midi_port = "off"
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
        assert!(cfg.presets.is_empty(), "presets example is commented out");
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

    // ---- Unit 01: config resolution -----------------------------------------

    /// A unique, empty scratch directory under the system temp dir. Tests
    /// create real files/dirs so `.exists()` semantics are exercised exactly as
    /// in production; the injected-home seam keeps `$HOME`/cwd untouched.
    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("lrex-test-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, body).expect("write file");
    }

    #[test]
    fn default_config_path_is_the_lrex_path() {
        let p = default_config_path().expect("home dir available");
        assert!(
            p.ends_with(".config/lrex/config.toml"),
            "default config path should be ~/.config/lrex/config.toml, got {}",
            p.display()
        );
    }

    #[test]
    fn explicit_config_beats_every_env_var_and_default() {
        let env = |k: &str| match k {
            "LREX_CONFIG" => Some("/from/lrex-env.toml".to_string()),
            "LUFS_RECORDER_CONFIG" => Some("/from/lufs-env.toml".to_string()),
            _ => None,
        };
        let got = resolve_path_with(Some(Path::new("/explicit.toml")), env, || None);
        assert_eq!(got, Some(PathBuf::from("/explicit.toml")));
    }

    #[test]
    fn lrex_config_env_beats_lufs_recorder_config_env() {
        let both = |k: &str| match k {
            "LREX_CONFIG" => Some("/from/lrex-env.toml".to_string()),
            "LUFS_RECORDER_CONFIG" => Some("/from/lufs-env.toml".to_string()),
            _ => None,
        };
        assert_eq!(
            resolve_path_with(None, both, || None),
            Some(PathBuf::from("/from/lrex-env.toml"))
        );

        // With LREX_CONFIG absent, the legacy var still works.
        let only_lufs =
            |k: &str| (k == "LUFS_RECORDER_CONFIG").then(|| "/from/lufs-env.toml".to_string());
        assert_eq!(
            resolve_path_with(None, only_lufs, || None),
            Some(PathBuf::from("/from/lufs-env.toml"))
        );

        // An empty LREX_CONFIG must not shadow a set LUFS_RECORDER_CONFIG.
        let empty_lrex = |k: &str| match k {
            "LREX_CONFIG" => Some(String::new()),
            "LUFS_RECORDER_CONFIG" => Some("/from/lufs-env.toml".to_string()),
            _ => None,
        };
        assert_eq!(
            resolve_path_with(None, empty_lrex, || None),
            Some(PathBuf::from("/from/lufs-env.toml"))
        );
    }

    #[test]
    fn existing_lrex_user_config_beats_existing_lufs_user_config() {
        let home = temp_dir("homecfg");
        let no_env = |_: &str| None;
        let h = || Some(home.clone());

        // Only a legacy config exists -> it is used (backwards compatibility).
        let legacy = home.join(".config/lufs-recorder/config.toml");
        write(&legacy, "device = \"legacy\"\n");
        assert_eq!(resolve_path_with(None, no_env, h), Some(legacy.clone()));

        // Adding the lrex config makes it win.
        let lrex = home.join(".config/lrex/config.toml");
        write(&lrex, "device = \"new\"\n");
        assert_eq!(resolve_path_with(None, no_env, h), Some(lrex.clone()));

        // Neither on disk -> the default (creation target) is the lrex path.
        std::fs::remove_file(&lrex).expect("remove lrex cfg");
        std::fs::remove_file(&legacy).expect("remove legacy cfg");
        assert_eq!(resolve_path_with(None, no_env, h), Some(lrex));

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn project_override_prefers_lrex_then_falls_back_to_lufs() {
        let dir = temp_dir("proj");
        let lufs = dir.join(".lufs-recorder.toml");
        let lrex = dir.join(".lrex.toml");

        // Legacy file alone is discovered.
        write(&lufs, "device = \"legacy\"\n");
        assert_eq!(project_override_path_in(&dir), Some(lufs.clone()));

        // .lrex.toml wins once present.
        write(&lrex, "device = \"new\"\n");
        assert_eq!(project_override_path_in(&dir), Some(lrex.clone()));

        // An empty directory has no override.
        let empty = temp_dir("proj-empty");
        assert_eq!(project_override_path_in(&empty), None);

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn legacy_project_override_is_still_parsed_and_applied_when_lrex_absent() {
        let dir = temp_dir("proj-apply");
        write(
            &dir.join(".lufs-recorder.toml"),
            "device = \"BlackHole 2ch\"\n",
        );

        let path = project_override_path_in(&dir).expect("legacy override discovered");
        let text = std::fs::read_to_string(&path).expect("read override");
        let over: ProjectOverride = toml::from_str(&text).expect("legacy override parses");
        let mut base = Config::default();
        over.apply_to(&mut base);
        assert_eq!(base.device, "BlackHole 2ch");
        assert_eq!(base.bit_depth, "24", "untouched fields keep base values");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Unit 02: named presets ---------------------------------------------

    fn base_with_inline_preset() -> Config {
        let mut c = Config::default();
        c.presets.insert(
            "dante".to_string(),
            PresetConfig {
                description: Some("Dante course audio".to_string()),
                override_fields: ProjectOverride {
                    rate: Some("48000".to_string()),
                    midi_port: Some("off".to_string()),
                    ..ProjectOverride::default()
                },
            },
        );
        c
    }

    #[test]
    fn inline_preset_table_parses_into_config() {
        let text = r#"
device = "default"
rate = "default"
bit_depth = "24"
out_dir = "~/x"
midi_port = "off"

[[tracks]]
name = "mic"
channels = [1, 2]

[presets.dante]
description = "Dante course audio"
rate = "48000"
midi_port = "off"
"#;
        let cfg: Config = toml::from_str(text).expect("config with inline preset parses");
        assert!(cfg.presets.contains_key("dante"));
        assert_eq!(
            cfg.presets["dante"].description.as_deref(),
            Some("Dante course audio")
        );
    }

    #[test]
    fn resolve_preset_from_inline_config_table() {
        let base = base_with_inline_preset();
        let resolved = resolve_preset_in(None, "dante", &base).expect("inline preset resolves");
        assert_eq!(resolved.rate, "48000");
        assert_eq!(resolved.midi_port, "off");
        // Untouched fields keep base values; the base config is not mutated.
        assert_eq!(resolved.bit_depth, "24");
        assert_eq!(base.rate, "default");
    }

    #[test]
    fn resolve_preset_from_file_and_lrex_wins_over_lufs() {
        let home = temp_dir("presets-home");
        write(
            &home.join(".config/lufs-recorder/presets/dante.toml"),
            "device = \"lufs-device\"\nrate = \"44100\"\n",
        );
        write(
            &home.join(".config/lrex/presets/dante.toml"),
            "device = \"lrex-device\"\nrate = \"48000\"\ndescription = \"from file\"\n",
        );

        let resolved = resolve_preset_in(Some(&home), "dante", &Config::default())
            .expect("file preset resolves");
        assert_eq!(resolved.device, "lrex-device");
        assert_eq!(resolved.rate, "48000");

        // Removing the lrex file falls back to the lufs-recorder preset.
        std::fs::remove_file(home.join(".config/lrex/presets/dante.toml"))
            .expect("remove lrex preset");
        let resolved = resolve_preset_in(Some(&home), "dante", &Config::default())
            .expect("legacy file preset resolves");
        assert_eq!(resolved.device, "lufs-device");
        assert_eq!(resolved.rate, "44100");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn unknown_preset_is_an_explicit_error_listing_known_presets() {
        let base = base_with_inline_preset();
        let err = resolve_preset_in(None, "nope", &base).expect_err("unknown preset must error");
        let msg = format!("{err:#}");
        assert!(msg.contains("unknown preset"), "got: {msg}");
        assert!(
            msg.contains("dante"),
            "error should list known presets; got: {msg}"
        );
    }

    #[test]
    fn preset_rejects_unknown_field() {
        // `deny_unknown_fields` + `flatten` must still reject a typo'd field in
        // a preset rather than silently ignoring it.
        let bad = "description = \"x\"\ndevice = \"y\"\nnot_a_field = \"z\"\n";
        let parsed: std::result::Result<PresetConfig, _> = toml::from_str(bad);
        assert!(parsed.is_err(), "a typo'd preset field must fail loudly");
    }

    #[test]
    fn list_available_presets_merges_files_and_inline_then_sorts() {
        let home = temp_dir("presets-list");
        write(
            &home.join(".config/lrex/presets/zulu.toml"),
            "description = \"Z\"\ndevice = \"z\"\n",
        );
        // A file named dante wins over the inline dante of the same name.
        write(
            &home.join(".config/lrex/presets/dante.toml"),
            "description = \"from file\"\ndevice = \"d\"\n",
        );
        let base = base_with_inline_preset();

        let listed = list_available_presets_in(Some(&home), &base);
        let names: Vec<&str> = listed.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["dante", "zulu"], "sorted, deduped by name");
        assert_eq!(listed[0].description.as_deref(), Some("from file"));

        let _ = std::fs::remove_dir_all(&home);
    }
}
