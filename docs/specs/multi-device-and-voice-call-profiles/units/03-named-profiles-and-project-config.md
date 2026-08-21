# Unit 03 — Named Profiles & Project-Scoped Config

*Status: implemented (this PR).*

## Objective
Add a user-authored named-profile concept (`auto_stop_minutes` +
`buffer_minutes` per profile) to `Config`, plus project-local config override
(`.lufs-recorder.toml` layered over the global config), and a pure function
turning a profile name into a concrete capture duration.

## Context
- Proposal's "Named profiles" subsection (shape, buffer semantics, project
  override)
- SPEC.md (this suite) §5 — the buffer-semantics interpretation confirmed with
  Daniel
- Code: `src/config.rs` (`ProfileConfig`, `Config::profiles`, `ProjectOverride`,
  `Config::load`, `resolve_profile_duration_secs`)

## Acceptance criteria
- [x] `Config` gained a `profiles: HashMap<String, ProfileConfig>` map
      (`#[serde(default)]`, so a config with no `[profiles.*]` table at all
      still parses exactly as before — verified by
      `old_config_without_profiles_table_still_parses` using the literal text
      of today's emitted config, not a paraphrase).
- [x] A pure function `resolve_profile_duration_secs(cfg, profile_name)` returns
      either `Ok(seconds)` or an explicit error naming the configured profiles
      — never a silent fallback to an undocumented default.
- [x] Config loading discovers an optional `.lufs-recorder.toml` from
      `std::env::current_dir()` and layers it over the already-resolved base
      config; only fields present in the project file override, everything
      else falls through. Verified both by unit test
      (`project_override_changes_only_named_fields`) and by a real CLI run
      from a temp project directory (the resolved device name in the resulting
      error message proved the override actually applied end-to-end, not just
      in an isolated test).
- [x] `#[serde(deny_unknown_fields)]` holds for both the base config (unchanged)
      and the new `ProjectOverride` type — a typo'd field in either fails
      loudly (`project_override_rejects_unknown_field`).
- [x] Existing configs with no `profiles` table and no project-local file
      load and behave exactly as before (additive) — same evidence as above.

## Interface contract
```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    pub auto_stop_minutes: f64,
    pub buffer_minutes: f64,
}
// Config gains: #[serde(default)] pub profiles: HashMap<String, ProfileConfig>

pub fn resolve_profile_duration_secs(cfg: &Config, profile_name: &str) -> anyhow::Result<f64>
// = (auto_stop_minutes + 2.0 * buffer_minutes) * 60.0 — symmetric buffer,
// measured from actual recording start (confirmed interpretation, SPEC.md §5).
```
`Config::load()`'s signature is unchanged (`Option<&Path> -> Result<(Config,
Option<PathBuf>)>`) — project-override discovery happens internally, so every
existing call site (`main.rs::load_config`) needed zero changes.

## Boundaries — do NOT touch
- `RecordPlan` / `resolve_plan()` / the capture loop (Unit 01 owns these; Unit
  03 only supplies a value through `resolve_profile_duration_secs`).
- Serve API / CLI argument parsing for `profile` (Unit 04 owns wiring the
  wire-level field to this unit's function).
- Verification (Unit 02).
- Unit 04 reads `Config.profiles` (e.g. to validate a profile name exists) but
  never writes to `Config` — any new field on `Config` for this feature belongs
  to Unit 03 alone.

## Output
- `src/config.rs`: `ProfileConfig`, `ProjectOverride` (+ `apply_to`),
  `Config::load` extended, `resolve_profile_duration_secs`, `validate()`
  extended to reject non-positive `auto_stop_minutes` / negative
  `buffer_minutes`, `default_config_toml()` documents the new `[profiles.*]`
  table (commented-out example).
- 9 new unit tests.
- Commit: `feat(config): named profiles + project-scoped config override`

## Verification
- `cargo test`: 9 dedicated tests, all passing.
- Manual CLI integration test (beyond unit tests): wrote a real
  `.lufs-recorder.toml` to a temp directory, ran the compiled binary with
  `--config <base>` from inside it, and confirmed both the device override and
  a `--profile standup` resolution flowed through correctly (the latter proven
  by the run reaching the device-resolution step at all, meaning
  `resolve_profile_duration_secs` succeeded first).
- `cargo clippy`/`fmt` clean.
