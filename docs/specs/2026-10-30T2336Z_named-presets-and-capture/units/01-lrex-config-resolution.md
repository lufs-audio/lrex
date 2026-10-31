# Unit 01 — lrex-config-resolution

## Objective
Add first-class support for `.lrex.toml` project overrides, `~/.config/lrex/config.toml` user configuration, and the `LREX_CONFIG` environment variable, while preserving 100% backwards-compatible fallback to their `.lufs-recorder` equivalents.

## Context
- `src/config.rs`: `resolve_path`, `project_override_path`, `PROJECT_OVERRIDE_FILENAME`, `default_config_toml`.
- `src/bin/lrex.rs` and `src/bin/lufs-recorder.rs`: dual binary entry points.
- `SPEC.md` §4.1: Config & Preset Hierarchy.
- `CONTRACT.md`: Config resolution rules.

## Acceptance criteria
- [ ] `project_override_path()` discovers `.lrex.toml` in the current working directory first. If `.lrex.toml` is absent, it falls back to `.lufs-recorder.toml`.
- [ ] `resolve_path(explicit)` resolves in order:
  1. `explicit` (`--config <path>`)
  2. `LREX_CONFIG` environment variable (if non-empty)
  3. `LUFS_RECORDER_CONFIG` environment variable (if non-empty)
  4. `~/.config/lrex/config.toml` (if it exists on disk)
  5. `~/.config/lufs-recorder/config.toml` (if it exists on disk)
  6. Default path `~/.config/lrex/config.toml` (when creating new or as fallback)
- [ ] `init-config` subcommand writes to `~/.config/lrex/config.toml` (creating parent directories if missing). If `~/.config/lufs-recorder/config.toml` already exists and `~/.config/lrex/config.toml` does not, it informs the user and does not unintentionally clobber the old file without confirmation.
- [ ] Existing tests in `src/config.rs` pass without regressions.
- [ ] New unit tests verify:
  - `.lrex.toml` takes priority over `.lufs-recorder.toml` when both exist in a directory.
  - `.lufs-recorder.toml` is still parsed and applied when `.lrex.toml` is absent.
  - `LREX_CONFIG` takes priority over `LUFS_RECORDER_CONFIG`.
  - `~/.config/lrex/config.toml` takes priority over `~/.config/lufs-recorder/config.toml`.

## Interface contract
```rust
// src/config.rs
pub const PROJECT_OVERRIDE_FILENAMES: &[&str] = &[".lrex.toml", ".lufs-recorder.toml"];
pub const CONFIG_DIR_NAMES: &[&str] = &["lrex", "lufs-recorder"];
pub const ENV_CONFIG_VARS: &[&str] = &["LREX_CONFIG", "LUFS_RECORDER_CONFIG"];

pub fn resolve_path(explicit: Option<&Path>) -> Option<PathBuf>;
fn project_override_path() -> Option<PathBuf>;
```
The signature of `Config::load(explicit: Option<&Path>) -> Result<(Config, Option<PathBuf>)>` remains unchanged so existing callers in `main.rs`, `cli.rs`, and `server.rs` require zero changes.

## Boundaries — do NOT touch
- `src/devices.rs` (device enumeration and selection)
- `src/record.rs` (capture loop and timing)
- `src/verify.rs` (take contract verification)
- `src/server.rs` and `src/tui/` (HTTP server and terminal UI)

## Output
- Modified: `src/config.rs`
- Tests added in `src/config.rs` (`mod tests`) covering the new resolution order.
- Commit message: `feat(config): support .lrex.toml and ~/.config/lrex with backwards compatibility`

## Verification
```bash
cargo test config::tests
cargo test
cargo run --bin lrex -- record --dry-run --json
```
