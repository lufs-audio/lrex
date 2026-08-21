//! `lrex` — the short, bplate-compliant invocation name (3-6 chars, lowercase,
//! no hyphens; see `lufs-audio/bplate` docs/units/07-lufs-primitive-cli-naming.md).
//! Introduced as an alias alongside `lufs-recorder` per that document's explicit
//! recommendation for existing daily-driver tools (alias, not a breaking
//! rename — muscle memory and any existing scripts keep working under the old
//! name too). Behavior is byte-identical to `lufs-recorder`.

fn main() -> std::process::ExitCode {
    lufs_recorder::run_cli()
}
