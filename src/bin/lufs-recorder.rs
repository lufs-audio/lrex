//! The full, descriptive invocation name — matches the repo/project identity,
//! kept for discoverability, docs, and any existing muscle memory. Behavior is
//! byte-identical to the `lrex` alias (see `src/bin/lrex.rs`); both call the
//! same `lufs_recorder::run_cli()`.

fn main() -> std::process::ExitCode {
    lufs_recorder::run_cli()
}
