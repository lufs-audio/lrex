//! The capture engine: single-device multichannel audio (+ MIDI) to a verified take.
//!
//! Real-time discipline (design suite `03-stack-decision`): the audio callback
//! does no file I/O and no allocation on the hot path — it de-interleaves the
//! requested channels and pushes them into a lock-free SPSC ring buffer. A
//! separate writer thread drains the ring to disk. Dropped frames are detected
//! from the callback capture timestamps and from ring-buffer overruns; a
//! non-zero count fails the contract.

use crate::clock::SessionClock;
use crate::config::Config;
use crate::error::ExitError;
use crate::manifest::{
    self, Captured, Manifest, MidiInfo, Requested, RequestedTrack, TrackInfo, Verification,
    SCHEMA_VERSION,
};
use crate::{cli, devices, midi, verify};

use anyhow::anyhow;
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Sample, SampleFormat};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

type AudioProd = <HeapRb<f32> as Split>::Prod;

/// Seconds of audio the ring buffer can hold before the writer must catch up.
const RING_SECONDS: usize = 8;

pub struct RecordOutcome {
    pub take_dir: PathBuf,
    pub manifest: Manifest,
    /// True if capture was stopped by the user (Ctrl-C) rather than reaching a
    /// requested duration. Lets the caller report exit 6 for an interrupted,
    /// unverifiable take instead of a plain contract violation.
    pub interrupted: bool,
}

struct RecordPlan {
    device_query: String,
    tracks: Vec<(String, Vec<u16>)>,
    want_rate: Option<u32>,
    bit_depth: String,
    midi_spec: String,
    out_dir: PathBuf,
    name: Option<String>,
    duration: Option<f64>,
}

fn resolve_plan(cfg: &Config, args: &cli::RecordArgs) -> crate::error::Result<RecordPlan> {
    let device_query = args.device.clone().unwrap_or_else(|| cfg.device.clone());

    let tracks: Vec<(String, Vec<u16>)> = if let Some(spec) = &args.channels {
        let groups = cli::parse_channel_groups(spec).map_err(ExitError::Other)?;
        groups
            .into_iter()
            .map(|g| (cli::default_track_name(&g), g))
            .collect()
    } else {
        cfg.tracks
            .iter()
            .map(|t| (t.name.clone(), t.channels.clone()))
            .collect()
    };

    let bit_depth = args
        .bit_depth
        .clone()
        .unwrap_or_else(|| cfg.bit_depth.clone());
    crate::config::validate_bit_depth(&bit_depth).map_err(ExitError::Other)?;

    let want_rate = args.rate.or_else(|| cfg.rate_hz());
    let midi_spec = args.midi.clone().unwrap_or_else(|| cfg.midi_port.clone());
    let out_dir = args.out.clone().unwrap_or_else(|| cfg.out_path());

    Ok(RecordPlan {
        device_query,
        tracks,
        want_rate,
        bit_depth,
        midi_spec,
        out_dir,
        name: args.name.clone(),
        duration: args.duration,
    })
}

fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "track".to_string()
    } else {
        s
    }
}

fn max_channel(tracks: &[(String, Vec<u16>)]) -> u16 {
    tracks
        .iter()
        .flat_map(|(_, ch)| ch.iter().copied())
        .max()
        .unwrap_or(0)
}

fn wav_spec(channels: u16, sample_rate: u32, bit_depth: &str) -> hound::WavSpec {
    let (bits, fmt) = match bit_depth {
        "16" => (16, hound::SampleFormat::Int),
        "32f" => (32, hound::SampleFormat::Float),
        _ => (24, hound::SampleFormat::Int), // "24"
    };
    hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: bits,
        sample_format: fmt,
    }
}

/// Pre-flight: resolve device + format (real exit codes) and print the plan
/// without capturing. `config_source` is the config file actually loaded (or
/// `None` for built-in defaults), reported so the active config is unambiguous.
pub fn dry_run(
    cfg: &Config,
    args: &cli::RecordArgs,
    json: bool,
    config_source: Option<&std::path::Path>,
) -> crate::error::Result<()> {
    let plan = resolve_plan(cfg, args)?;
    let device =
        devices::resolve_input_device(&plan.device_query).map_err(ExitError::DeviceUnavailable)?;
    let dev_name = device.name().unwrap_or_else(|_| plan.device_query.clone());
    let mc = max_channel(&plan.tracks);
    let chosen = devices::choose_input_config(&device, plan.want_rate, mc)
        .map_err(ExitError::FormatUnsupported)?;

    let midi_ports: Vec<String> = if plan.midi_spec.eq_ignore_ascii_case("off") {
        vec![]
    } else {
        let want_all = plan.midi_spec.eq_ignore_ascii_case("all");
        let needle = plan.midi_spec.to_lowercase();
        midi::list_ports()
            .unwrap_or_default()
            .into_iter()
            .filter(|n| want_all || n.to_lowercase().contains(&needle))
            .collect()
    };

    if json {
        let payload = serde_json::json!({
            "dry_run": true,
            "device": dev_name,
            "sample_rate": chosen.sample_rate,
            "sample_format": devices::sample_format_name(chosen.sample_format),
            "device_channels": chosen.channels,
            "bit_depth": plan.bit_depth,
            "tracks": plan.tracks.iter().map(|(n, ch)| serde_json::json!({"name": n, "channels": ch})).collect::<Vec<_>>(),
            "midi_ports": midi_ports,
            "out_dir": plan.out_dir,
            "duration_s": plan.duration,
            "config": config_source.map(|p| p.display().to_string()),
        });
        println!("{payload}");
    } else {
        println!("dry-run OK — would record:");
        match config_source {
            Some(p) => println!("  config : {}", p.display()),
            None => println!("  config : (built-in defaults)"),
        }
        println!(
            "  device : {dev_name} ({} ch @ {} Hz, {})",
            chosen.channels,
            chosen.sample_rate,
            devices::sample_format_name(chosen.sample_format)
        );
        for (n, ch) in &plan.tracks {
            println!(
                "  track  : {n} <- channels {ch:?} -> {}.wav ({} bit)",
                slug(n),
                plan.bit_depth
            );
        }
        if midi_ports.is_empty() {
            println!("  midi   : off");
        } else {
            println!("  midi   : {midi_ports:?}");
        }
        println!("  out    : {}", plan.out_dir.display());
    }
    Ok(())
}

struct TrackWriter {
    channels: usize,
    writer: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    peak: f32,
    sumsq: f64,
    frames: u64,
    bit_depth: String,
}

fn dbfs(x: f32) -> f32 {
    if x <= 0.0 {
        -f32::INFINITY
    } else {
        20.0 * x.log10()
    }
}

fn write_sample(
    w: &mut hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    bit_depth: &str,
    v: f32,
) -> anyhow::Result<()> {
    let c = v.clamp(-1.0, 1.0);
    match bit_depth {
        "16" => w.write_sample((c * 32767.0) as i16)?,
        "32f" => w.write_sample(c)?,
        _ => w.write_sample((c * 8_388_607.0) as i32)?, // 24-bit
    }
    Ok(())
}

/// Per-track final stats: (frames, peak_dbfs, rms_dbfs).
struct WriterStats {
    per_track: Vec<(u64, f32, f32)>,
    frames: u64,
}

#[allow(clippy::too_many_arguments)]
fn build_input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    total_dev_channels: usize,
    selected_indices: Vec<usize>,
    mut prod: AudioProd,
    clock: SessionClock,
    audio_t0_ns: Arc<AtomicU64>,
    latency_frames: Arc<AtomicU64>,
    xruns: Arc<AtomicU64>,
    sample_rate: u32,
) -> std::result::Result<cpal::Stream, String>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let mut first = true;
    let mut prev_capture: Option<cpal::StreamInstant> = None;
    let sr = sample_rate as f64;
    let err_xruns = xruns.clone();

    let data_cb = move |data: &[T], info: &cpal::InputCallbackInfo| {
        let frames = data.len().checked_div(total_dev_channels).unwrap_or(0);
        let ts = info.timestamp();
        if first {
            first = false;
            audio_t0_ns.store(clock.now_ns() as u64, Ordering::Relaxed);
            if let Some(d) = ts.callback.duration_since(&ts.capture) {
                latency_frames.store((d.as_secs_f64() * sr) as u64, Ordering::Relaxed);
            }
        } else if let Some(prev) = prev_capture.as_ref() {
            if let Some(delta) = ts.capture.duration_since(prev) {
                let expected = frames as f64 / sr;
                if expected > 0.0 && delta.as_secs_f64() > expected * 1.5 {
                    let dropped = ((delta.as_secs_f64() - expected) * sr).round();
                    if dropped > 0.0 {
                        xruns.fetch_add(dropped as u64, Ordering::Relaxed);
                    }
                }
            }
        }
        prev_capture = Some(ts.capture);

        for f in 0..frames {
            let base = f * total_dev_channels;
            for &idx in &selected_indices {
                let v: f32 = f32::from_sample(data[base + idx]);
                if prod.try_push(v).is_err() {
                    // Writer fell behind: a ring-buffer overrun is a dropped frame.
                    xruns.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    };

    let err_cb = move |e: cpal::StreamError| {
        eprintln!("lufs-recorder: stream error: {e}");
        err_xruns.fetch_add(1, Ordering::Relaxed);
    };

    device
        .build_input_stream(config, data_cb, err_cb, None)
        .map_err(|e| format!("building input stream: {e}"))
}

/// Capture a take. Always writes `take.json` (even for an unverified take, so a
/// partial is inspectable); the caller maps `verified` to the exit code.
pub fn run(
    cfg: &Config,
    args: &cli::RecordArgs,
    json: bool,
) -> crate::error::Result<RecordOutcome> {
    let plan = resolve_plan(cfg, args)?;

    let device =
        devices::resolve_input_device(&plan.device_query).map_err(ExitError::DeviceUnavailable)?;
    let dev_name = device.name().unwrap_or_else(|_| plan.device_query.clone());
    let mc = max_channel(&plan.tracks);
    let chosen = devices::choose_input_config(&device, plan.want_rate, mc)
        .map_err(ExitError::FormatUnsupported)?;

    // Flat, 0-based indices of the channels to keep, in track order.
    let mut selected_indices: Vec<usize> = Vec::new();
    for (_, chans) in &plan.tracks {
        for &c in chans {
            if c > chosen.channels {
                return Err(ExitError::FormatUnsupported(format!(
                    "requested channel {c} but device provides {} channels",
                    chosen.channels
                )));
            }
            selected_indices.push((c - 1) as usize);
        }
    }
    let total_selected: usize = selected_indices.len();

    // Create the timestamped take directory.
    let created_iso = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H%M%S").to_string();
    let folder = match &plan.name {
        Some(label) => format!("{stamp}_{}", slug(label)),
        None => stamp,
    };
    let take_dir = plan.out_dir.join(folder);
    std::fs::create_dir_all(&take_dir)
        .map_err(|e| ExitError::Other(anyhow!("creating take dir {}: {e}", take_dir.display())))?;

    // One WAV writer per track.
    let mut track_writers: Vec<TrackWriter> = Vec::new();
    let mut track_files: Vec<String> = Vec::new();
    for (name, chans) in &plan.tracks {
        let file = format!("{}.wav", slug(name));
        let spec = wav_spec(chans.len() as u16, chosen.sample_rate, &plan.bit_depth);
        let writer = hound::WavWriter::create(take_dir.join(&file), spec)
            .map_err(|e| ExitError::Other(anyhow!("creating {file}: {e}")))?;
        track_writers.push(TrackWriter {
            channels: chans.len(),
            writer,
            peak: 0.0,
            sumsq: 0.0,
            frames: 0,
            bit_depth: plan.bit_depth.clone(),
        });
        track_files.push(file);
    }

    // Ring buffer sized to RING_SECONDS of selected audio.
    let ring_cap = (total_selected.max(1)) * (chosen.sample_rate as usize) * RING_SECONDS;
    let (prod, mut cons) = HeapRb::<f32>::new(ring_cap).split();

    // Shared state.
    let xruns = Arc::new(AtomicU64::new(0));
    let audio_t0_ns = Arc::new(AtomicU64::new(0));
    let latency_frames = Arc::new(AtomicU64::new(0));
    let frames_written = Arc::new(AtomicU64::new(0));
    let capturing = Arc::new(AtomicBool::new(true));
    let running = Arc::new(AtomicBool::new(true));

    // Session clock and MIDI arm (before audio, so t0 covers both).
    let clock = SessionClock::start();
    let midi_capture = midi::arm(&plan.midi_spec, clock).map_err(ExitError::DeviceUnavailable)?;
    let armed_ports = midi_capture
        .as_ref()
        .map(|c| c.ports.clone())
        .unwrap_or_default();

    // Writer thread — drains the ring to disk, tracks peak/RMS.
    let writer_capturing = capturing.clone();
    let writer_frames = frames_written.clone();
    let writer_handle = std::thread::spawn(move || -> anyhow::Result<WriterStats> {
        let mut frame_buf: Vec<f32> = Vec::with_capacity(total_selected.max(1));
        loop {
            let mut drained_any = false;
            while let Some(v) = cons.try_pop() {
                drained_any = true;
                frame_buf.push(v);
                if frame_buf.len() == total_selected {
                    let mut off = 0usize;
                    for tw in track_writers.iter_mut() {
                        for j in 0..tw.channels {
                            let s = frame_buf[off + j];
                            write_sample(&mut tw.writer, &tw.bit_depth, s)?;
                            let a = s.abs();
                            if a > tw.peak {
                                tw.peak = a;
                            }
                            tw.sumsq += (s as f64) * (s as f64);
                        }
                        tw.frames += 1;
                        off += tw.channels;
                    }
                    writer_frames.fetch_add(1, Ordering::Relaxed);
                    frame_buf.clear();
                }
            }
            if !writer_capturing.load(Ordering::Relaxed) && cons.is_empty() {
                break;
            }
            if !drained_any {
                std::thread::sleep(Duration::from_millis(4));
            }
        }

        let mut per_track = Vec::with_capacity(track_writers.len());
        let mut min_frames = u64::MAX;
        for tw in track_writers {
            let frames = tw.frames;
            min_frames = min_frames.min(frames);
            let rms = if frames == 0 {
                0.0
            } else {
                (tw.sumsq / (frames as f64 * tw.channels as f64)).sqrt() as f32
            };
            let peak_dbfs = dbfs(tw.peak);
            let rms_dbfs = dbfs(rms);
            tw.writer.finalize()?;
            per_track.push((frames, peak_dbfs, rms_dbfs));
        }
        Ok(WriterStats {
            per_track,
            frames: if min_frames == u64::MAX {
                0
            } else {
                min_frames
            },
        })
    });

    // Build + start the audio stream.
    let stream_config = cpal::StreamConfig {
        channels: chosen.channels,
        sample_rate: cpal::SampleRate(chosen.sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };
    let total_dev_channels = chosen.channels as usize;

    let stream = match chosen.sample_format {
        SampleFormat::F32 => build_input_stream::<f32>(
            &device,
            &stream_config,
            total_dev_channels,
            selected_indices.clone(),
            prod,
            clock,
            audio_t0_ns.clone(),
            latency_frames.clone(),
            xruns.clone(),
            chosen.sample_rate,
        ),
        SampleFormat::I16 => build_input_stream::<i16>(
            &device,
            &stream_config,
            total_dev_channels,
            selected_indices.clone(),
            prod,
            clock,
            audio_t0_ns.clone(),
            latency_frames.clone(),
            xruns.clone(),
            chosen.sample_rate,
        ),
        SampleFormat::I32 => build_input_stream::<i32>(
            &device,
            &stream_config,
            total_dev_channels,
            selected_indices.clone(),
            prod,
            clock,
            audio_t0_ns.clone(),
            latency_frames.clone(),
            xruns.clone(),
            chosen.sample_rate,
        ),
        SampleFormat::U16 => build_input_stream::<u16>(
            &device,
            &stream_config,
            total_dev_channels,
            selected_indices.clone(),
            prod,
            clock,
            audio_t0_ns.clone(),
            latency_frames.clone(),
            xruns.clone(),
            chosen.sample_rate,
        ),
        SampleFormat::I8 => build_input_stream::<i8>(
            &device,
            &stream_config,
            total_dev_channels,
            selected_indices.clone(),
            prod,
            clock,
            audio_t0_ns.clone(),
            latency_frames.clone(),
            xruns.clone(),
            chosen.sample_rate,
        ),
        SampleFormat::U8 => build_input_stream::<u8>(
            &device,
            &stream_config,
            total_dev_channels,
            selected_indices,
            prod,
            clock,
            audio_t0_ns.clone(),
            latency_frames.clone(),
            xruns.clone(),
            chosen.sample_rate,
        ),
        other => {
            return Err(ExitError::FormatUnsupported(format!(
                "unsupported device sample format {other:?}"
            )))
        }
    }
    .map_err(ExitError::FormatUnsupported)?;

    // Ctrl-C stops cleanly (a partial still finalizes + verifies honestly).
    {
        let r = running.clone();
        // set_handler errors only if already set; ignore so repeat runs in one
        // process (e.g. tests) don't panic.
        let _ = ctrlc::set_handler(move || r.store(false, Ordering::SeqCst));
    }

    stream
        .play()
        .map_err(|e| ExitError::Other(anyhow!("starting stream: {e}")))?;

    if !json {
        match plan.duration {
            Some(d) => eprintln!(
                "lufs-recorder: recording {d:.1}s to {} …",
                take_dir.display()
            ),
            None => eprintln!(
                "lufs-recorder: recording to {} … (Ctrl-C to stop)",
                take_dir.display()
            ),
        }
    }

    let start = Instant::now();
    let mut last_progress = Instant::now();
    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }
        if let Some(d) = plan.duration {
            if start.elapsed().as_secs_f64() >= d {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
        if json && last_progress.elapsed() >= Duration::from_millis(500) {
            last_progress = Instant::now();
            let payload = serde_json::json!({
                "event": "progress",
                "elapsed_s": start.elapsed().as_secs_f64(),
                "frames": frames_written.load(Ordering::Relaxed),
                "xruns": xruns.load(Ordering::Relaxed),
            });
            println!("{payload}");
        }
    }

    let interrupted = !running.load(Ordering::SeqCst);

    // Stop capture, drain, finalize.
    drop(stream);
    capturing.store(false, Ordering::Relaxed);
    let stats = writer_handle
        .join()
        .map_err(|_| ExitError::Other(anyhow!("writer thread panicked")))?
        .map_err(ExitError::Other)?;

    // Finish MIDI.
    let (midi_info, midi_events) = if let Some(cap) = midi_capture {
        let events = cap.finish();
        if events.is_empty() {
            (None, 0u64)
        } else {
            let counts = midi::write_smf(&take_dir.join("capture.mid"), &events)
                .map_err(ExitError::Other)?;
            (
                Some(MidiInfo {
                    file: "capture.mid".to_string(),
                    ports: armed_ports.clone(),
                    events: counts.events,
                    note_ons: counts.note_ons,
                    note_offs: counts.note_offs,
                }),
                counts.events,
            )
        }
    } else {
        (None, 0)
    };

    // Assemble the manifest.
    let requested = Requested {
        device: plan.device_query.clone(),
        tracks: plan
            .tracks
            .iter()
            .map(|(n, ch)| RequestedTrack {
                name: n.clone(),
                channels: ch.clone(),
            })
            .collect(),
        rate: plan.want_rate.unwrap_or(chosen.sample_rate),
        bit_depth: plan.bit_depth.clone(),
        midi: armed_ports.clone(),
        duration_s: plan.duration,
    };
    let take_id = manifest::take_id(&requested, &created_iso);

    let frames = stats.frames;
    let captured = Captured {
        device: dev_name,
        rate: chosen.sample_rate,
        bit_depth: plan.bit_depth.clone(),
        channels: total_selected as u16,
        frames,
        duration_s: if chosen.sample_rate > 0 {
            frames as f64 / chosen.sample_rate as f64
        } else {
            0.0
        },
        xruns: xruns.load(Ordering::Relaxed),
        audio_t0_monotonic_ns: audio_t0_ns.load(Ordering::Relaxed) as u128,
        input_latency_frames: latency_frames.load(Ordering::Relaxed),
        midi_events,
        av_offset_ms: audio_t0_ns.load(Ordering::Relaxed) as f64 / 1e6,
    };

    let tracks_info: Vec<TrackInfo> = plan
        .tracks
        .iter()
        .zip(track_files.iter())
        .zip(stats.per_track.iter())
        .map(|(((_, ch), file), (_frames, peak, rms))| TrackInfo {
            file: file.clone(),
            channels: ch.clone(),
            peak_dbfs: *peak,
            rms_dbfs: *rms,
        })
        .collect();

    let mut mfst = Manifest {
        schema_version: SCHEMA_VERSION,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        take_id,
        created: created_iso,
        requested,
        captured,
        tracks: tracks_info,
        midi: midi_info,
        verification: Verification {
            verified: false,
            checks: vec![],
        },
    };
    mfst.verification = verify::run(&take_dir, &mfst);

    let json_text = serde_json::to_string_pretty(&mfst).map_err(|e| ExitError::Other(e.into()))?;
    std::fs::write(take_dir.join("take.json"), json_text)
        .map_err(|e| ExitError::Other(e.into()))?;

    Ok(RecordOutcome {
        take_dir,
        manifest: mfst,
        interrupted,
    })
}
