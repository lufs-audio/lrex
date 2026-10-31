# Unit 02 — named-config-presets

## Objective
Add `--preset <name>` to the CLI (`record` and `serve`), allowing users to switch configurations by name (resolving from `~/.config/lrex/presets/<name>.toml`, `~/.config/lufs-recorder/presets/<name>.toml`, or an inline `[presets.<name>]` table in the active config), plus an `lrex presets` discovery command.

## Context
- `src/cli.rs`: `RecordArgs`, `Commands`.
- `src/config.rs`: `Config`, `ProjectOverride`, `load()`.
- `src/record.rs`: `resolve_plan()`.
- `SPEC.md` §4.1: Config & Preset Hierarchy.

## Acceptance criteria
- [ ] `--preset <NAME>` added as an optional argument to `RecordArgs` in `src/cli.rs`.
- [ ] A new CLI subcommand `lrex presets` (alias `lrex presets list`) lists all available presets found in:
  1. `~/.config/lrex/presets/*.toml` and `~/.config/lufs-recorder/presets/*.toml`
  2. The `[presets]` table in the active user config (if present)
- [ ] Preset resolution order for `--preset <name>`:
  1. File `~/.config/lrex/presets/<name>.toml`
  2. File `~/.config/lufs-recorder/presets/<name>.toml`
  3. Inline entry `[presets.<name>]` in the active base config
  4. If not found, exits with code 3 (unavailable configuration target) and prints a helpful error listing all discovered presets.
- [ ] `--preset <name>` and `--config <file>` are mutually exclusive; clap emits an error if both are provided.
- [ ] Any CLI flags (e.g. `--rate 48000` or `--duration 30`) passed alongside `--preset <name>` cleanly override the preset's values, maintaining standard CLI flag precedence.
- [ ] Unit tests in `src/config.rs` and `src/cli.rs` verify:
  - Preset resolution from file path and from config table.
  - Unknown preset returns an explicit descriptive error listing known presets.
  - Mutual exclusion between `--preset` and `--config`.

## Interface contract
```rust
// src/config.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetConfig {
    pub description: Option<String>,
    #[serde(flatten)]
    pub override_fields: ProjectOverride,
}

// Config gains:
// #[serde(default)]
// pub presets: HashMap<String, PresetConfig>,

pub fn list_available_presets(base_cfg: &Config) -> Vec<PresetSummary>;
pub fn resolve_preset(name: &str, base_cfg: &Config) -> anyhow::Result<Config>;

// src/cli.rs RecordArgs gains:
// #[arg(long, conflicts_with = "config", help = "Named preset configuration to load")]
// pub preset: Option<String>,
```

## Boundaries — do NOT touch
- `src/devices.rs` (device enumeration)
- `src/verify.rs` (contract verification)
- Audio capture ringbuffers and writers in `src/record.rs`

## Output
- Modified: `src/cli.rs`, `src/config.rs`, `src/record.rs`
- Unit tests added in `src/config.rs` and `src/cli.rs`.
- Commit message: `feat(cli): add --preset flag and presets listing subcommand`

## Verification
```bash
cargo test
cargo run --bin lrex -- presets
cargo run --bin lrex -- record --preset non-existent --dry-run
```
