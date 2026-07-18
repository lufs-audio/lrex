//! lufs-recorder — universal, agent-first audio + MIDI recorder.
//!
//! v0.2: single-device multichannel audio + MIDI capture with maxpatch parity
//! (mic on 1-2, piano on 9-10, 24-bit, Nord Stage 3), a config file, and inline
//! verification. `works` means proven correct, not merely exited 0 — every take
//! is checked against the contract before it is declared good (see CONTRACT.md
//! and the design suite in danialrami/agent-knowledge : docs/product/lufs-recorder).

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
mod verify;

use clap::Parser;
use cli::{Cli, Command};
use config::Config;
use error::{code, ExitError};
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();
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
        Command::Serve { port, frontend } => {
            let (cfg, cfg_path) = load_config(cli)?;
            server::serve(cfg, cfg_path, *port, frontend.clone(), cli.json)
        }
        Command::InitConfig { out, force } => cmd_init_config(cli, out.clone(), *force),
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
        println!("edit `device`, `midi_port`, and `out_dir` for this machine, then `lufs-recorder devices` to confirm names.");
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
        eprintln!("lufs-recorder: {e}");
    }
}
