//! lufs-recorder / lrex — universal, agent-first audio + MIDI recorder.
//!
//! One implementation, two invocation names (`lufs-recorder` for discoverability
//! and repo identity, `lrex` as the short, bplate-compliant alias for daily
//! typing — see `src/bin/*.rs` and `lufs-audio/bplate` `docs/units/07-lufs-primitive-cli-naming.md`,
//! which explicitly recommends an alias over a breaking rename for a daily-driver
//! tool like this one). `works` means proven correct, not merely exited 0 —
//! every take is checked against the contract before it is declared good (see
//! CONTRACT.md and the design suite in `lufs-audio/kb` : docs/product/lufs-recorder).

mod cli;
mod clock;
mod config;
mod devices;
mod error;
mod fixture;
mod manifest;
mod midi;
mod record;
mod server;
mod tui;
mod verify;

use clap::{CommandFactory, FromArgMatches};
use cli::{Cli, Command};
use config::Config;
use error::{code, ExitError};
use std::process::ExitCode;

/// The name this binary was actually invoked as (argv[0]'s basename) — so
/// `--help`/usage output and diagnostic messages say "lrex" when run as `lrex`
/// and "lufs-recorder" when run under that name, rather than hardcoding one.
/// Falls back to "lufs-recorder" if argv[0] is somehow unavailable/unparsable
/// (never panics on a missing or malformed argv[0]).
pub fn invoked_name() -> String {
    std::env::args_os()
        .next()
        .and_then(|p| {
            std::path::Path::new(&p)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "lufs-recorder".to_string())
}

/// Shared entry point for every alias binary this crate ships (`src/bin/*.rs`)
/// — identical behavior regardless of which name invoked it.
pub fn run_cli() -> ExitCode {
    // clap's Command::name()/bin_name() want a `'static str`, not an owned
    // String; leaking is the standard fix for a value computed once and kept
    // for the life of a short-lived CLI process (one small, one-time leak,
    // not a per-call/per-loop leak).
    let prog: &'static str = Box::leak(invoked_name().into_boxed_str());
    // Drive clap manually (instead of the `Cli::parse()` convenience) so the
    // displayed command name matches how the binary was actually invoked,
    // instead of a name baked in at compile time.
    let cmd = Cli::command().name(prog).bin_name(prog);
    let matches = cmd.get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => e.exit(),
    };
    let json = cli.json;

    let result = run(&cli);

    match result {
        Ok(()) => ExitCode::from(code::OK),
        Err(e) => {
            report_error(&e, json);
            ExitCode::from(e.code())
        }
    }
}

fn run(cli: &Cli) -> error::Result<()> {
    match &cli.command {
        Command::Devices => cmd_devices(cli.json),
        Command::Record(args) => cmd_record(cli, args),
        Command::Verify { take_dir } => cmd_verify(take_dir, cli.json),
        Command::Selftest => cmd_selftest(cli.json),
        Command::Serve {
            host,
            port,
            frontend,
        } => {
            let (cfg, cfg_path) = load_config(cli)?;
            server::serve(
                cfg,
                cfg_path,
                host.clone(),
                *port,
                frontend.clone(),
                cli.json,
            )
        }
        Command::InitConfig { out, force } => cmd_init_config(cli, out.clone(), *force),
        Command::Tui { host, port, theme } => tui::run_tui(host.clone(), *port, *theme),
    }
}

fn cmd_selftest(json: bool) -> error::Result<()> {
    let report = fixture::run_all().map_err(ExitError::Other)?;
    if json {
        let payload = serde_json::json!({
            "selftest": true,
            "passed": report.passed(),
            "checks": report.checks,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).map_err(|e| ExitError::Other(e.into()))?
        );
    } else {
        println!(
            "{}",
            if report.passed() {
                "selftest PASSED"
            } else {
                "selftest FAILED"
            }
        );
        for c in &report.checks {
            let mark = if c.ok {
                "ok"
            } else if c.gating {
                "FAIL"
            } else {
                "warn"
            };
            let detail = c.detail.as_deref().unwrap_or("");
            println!("  [{mark}] {} {detail}", c.name);
        }
    }
    if report.passed() {
        Ok(())
    } else {
        Err(ExitError::ContractViolation("selftest failed".to_string()))
    }
}

fn load_config(cli: &Cli) -> error::Result<(Config, Option<std::path::PathBuf>)> {
    Config::load(cli.config.as_deref()).map_err(ExitError::Other)
}

fn cmd_devices(json: bool) -> error::Result<()> {
    let audio = devices::list_input_devices().map_err(ExitError::Other)?;
    let midi_ports = midi::list_ports().unwrap_or_default();

    if json {
        let payload = serde_json::json!({
            "audio_input_devices": audio,
            "midi_input_ports": midi_ports,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).map_err(|e| ExitError::Other(e.into()))?
        );
    } else {
        println!("Audio input devices:");
        if audio.is_empty() {
            println!("  (none found)");
        }
        for d in &audio {
            let star = if d.is_default { " *default*" } else { "" };
            println!(
                "  {}{star}\n     {} in ch, default {} Hz {}, rates {}-{} Hz, formats {}",
                d.name,
                d.max_input_channels,
                d.default_sample_rate,
                d.default_sample_format,
                d.sample_rate_range[0],
                d.sample_rate_range[1],
                d.sample_formats.join("/"),
            );
        }
        println!("\nMIDI input ports:");
        if midi_ports.is_empty() {
            println!("  (none found)");
        }
        for p in &midi_ports {
            println!("  {p}");
        }
    }
    Ok(())
}

fn cmd_record(cli: &Cli, args: &cli::RecordArgs) -> error::Result<()> {
    let (cfg, cfg_path) = load_config(cli)?;

    if args.dry_run {
        return record::dry_run(&cfg, args, cli.json, cfg_path.as_deref());
    }

    let outcome = record::run(&cfg, args, cli.json)?;
    report_take(
        &outcome.manifest,
        &outcome.take_dir.display().to_string(),
        cli.json,
    )?;

    if outcome.manifest.verification.verified {
        Ok(())
    } else if outcome.interrupted {
        Err(ExitError::Interrupted(format!(
            "interrupted before a valid take: {}",
            outcome.take_dir.display()
        )))
    } else {
        Err(ExitError::ContractViolation(format!(
            "take FAILED verification: {}",
            outcome.take_dir.display()
        )))
    }
}

fn cmd_verify(take_dir: &std::path::Path, json: bool) -> error::Result<()> {
    let (manifest, verification) = verify::verify_dir(take_dir).map_err(ExitError::Other)?;
    report_take(&manifest, &take_dir.display().to_string(), json)?;
    if verification.verified {
        Ok(())
    } else {
        Err(ExitError::ContractViolation(format!(
            "take FAILED verification: {}",
            take_dir.display()
        )))
    }
}

fn cmd_init_config(cli: &Cli, out: Option<std::path::PathBuf>, force: bool) -> error::Result<()> {
    let path = out
        .or_else(|| cli.config.clone())
        .or_else(config::default_config_path)
        .ok_or_else(|| ExitError::Other(anyhow::anyhow!("could not determine a config path")))?;

    if path.exists() && !force {
        return Err(ExitError::Other(anyhow::anyhow!(
            "{} already exists (use --force to overwrite)",
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ExitError::Other(e.into()))?;
    }
    std::fs::write(&path, config::default_config_toml()).map_err(|e| ExitError::Other(e.into()))?;

    if cli.json {
        println!("{}", serde_json::json!({"wrote_config": path}));
    } else {
        println!(
            "wrote default (maxpatch-parity) config to {}",
            path.display()
        );
        println!(
            "edit `device`, `midi_port`, and `out_dir` for this machine, then `{} devices` to confirm names.",
            invoked_name()
        );
    }
    Ok(())
}

fn report_take(manifest: &manifest::Manifest, take_dir: &str, json: bool) -> error::Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(manifest).map_err(|e| ExitError::Other(e.into()))?
        );
        return Ok(());
    }

    let v = &manifest.verification;
    println!(
        "{} — {} ({:.2}s, {} Hz, {} xruns)",
        if v.verified { "VERIFIED" } else { "FAILED" },
        take_dir,
        manifest.captured.duration_s,
        manifest.captured.rate,
        manifest.captured.xruns,
    );
    for t in &manifest.tracks {
        println!("  {} — peak {:.1} dBFS", t.file, t.peak_dbfs);
    }
    if let Some(m) = &manifest.midi {
        let synth = if m.synthesized_note_offs > 0 {
            format!(", {} held note(s) closed at end", m.synthesized_note_offs)
        } else {
            String::new()
        };
        println!(
            "  {} — {} events ({} on / {} off{synth})",
            m.file, m.events, m.note_ons, m.note_offs
        );
    }
    for c in &v.checks {
        if !c.ok {
            let tag = if c.gating { "FAIL" } else { "warn" };
            let detail = c.detail.as_deref().unwrap_or("");
            println!("  [{tag}] {} {detail}", c.name);
        }
    }
    Ok(())
}

fn report_error(e: &ExitError, json: bool) {
    if json {
        let payload = serde_json::json!({
            "error": e.kind(),
            "message": e.to_string(),
            "exit_code": e.code(),
        });
        println!("{payload}");
    } else {
        eprintln!("{}: {e}", invoked_name());
    }
}
