//! Argument parsing and command dispatch.
//!
//! Hand-rolled rather than `clap`: four subcommands do not justify a dependency, and this
//! crate's dependency graph is load-bearing evidence rather than a convenience.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use vrecnu_engine::{RecordRequest, Session, SessionMachine};
use vrecnu_publish_server::{PublishAssets, REGISTERED_PORTS};
use vrecnu_shell_api::{EngineEvent, RecorderEngine, ShellServices};
use vrecnu_types::{BubbleSpec, CaptureSourceId};

use crate::shell::CliShell;

/// A CLI-level failure. Distinct from `AppError` — these are usage problems, not engine ones.
#[derive(Debug)]
pub enum CliError {
    /// The user asked for something that does not exist.
    UnknownCommand(String),
    /// A required argument was missing.
    MissingArgument(&'static str),
    /// An argument would not parse.
    BadArgument {
        /// Which flag.
        flag: &'static str,
        /// What was supplied.
        value: String,
    },
    /// The engine refused.
    Engine(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCommand(cmd) => {
                write!(
                    f,
                    "unknown command {cmd:?}. Run `vrecnu-cli help` for usage."
                )
            }
            Self::MissingArgument(name) => write!(f, "missing required argument --{name}"),
            Self::BadArgument { flag, value } => {
                write!(f, "could not parse --{flag} value {value:?}")
            }
            Self::Engine(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for CliError {}

const USAGE: &str = "\
vrecnu-cli — headless vrecnu

USAGE:
    vrecnu-cli <COMMAND>

COMMANDS:
    sources                     List capturable screens and windows
    cameras                     List available cameras
    record --seconds <N>        Record for N seconds, then encode
        [--source <ID>]         Which screen or window (default: primary display)
        [--out <PATH>]          Output file (default: vrecnu-recording.mp4)
        [--fps <N>]             Frame rate (default: 30)
        [--mic] [--system]      Include microphone / system audio
        [--camera [ID]]         Burn a camera bubble into the recording
                                (no ID: the system default camera)
        [--bubble]              Also show the floating bubble on screen
    publish <FILE>              Serve FILE to the publish page and wait for a share link
    doctor                      Report what this build can do on this machine
    help                        Show this message

ENVIRONMENT:
    VRECNU_LOG                  Log filter, e.g. `debug` or `vrecnu_capture=trace`
    VRECNU_FFMPEG               Path to an ffmpeg binary, overriding the bundled one

This binary exists to prove vrecnu's engine has no shell dependency (ADR-006). It is built
in CI on every commit; keep it working.
";

/// Options for `record`.
#[derive(Debug, PartialEq, Eq)]
pub struct RecordOptions {
    /// How long to capture.
    pub seconds: u32,
    /// Which source, or `None` for the primary display.
    pub source: Option<String>,
    /// Where to write.
    pub out: String,
    /// Target frame rate.
    pub fps: u32,
    /// Include the microphone.
    pub mic: bool,
    /// Include system audio.
    pub system: bool,
    /// Camera to burn in. `Some("")` is the system default; `None` records no camera.
    pub camera: Option<String>,
    /// Show the floating bubble window as well as burning the camera in.
    pub bubble: bool,
}

/// A parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// List capture sources.
    Sources,
    /// List cameras.
    Cameras,
    /// Record for a fixed duration.
    Record(RecordOptions),
    /// Serve a file to the publish page.
    Publish {
        /// File to publish.
        path: String,
    },
    /// Report capabilities.
    Doctor,
    /// Show usage.
    Help,
}

/// Parses argv (without the program name).
pub fn parse(args: &[String]) -> Result<Command, CliError> {
    let Some(first) = args.first() else {
        return Ok(Command::Help);
    };

    match first.as_str() {
        "help" | "--help" | "-h" => Ok(Command::Help),
        "sources" => Ok(Command::Sources),
        "cameras" => Ok(Command::Cameras),
        "doctor" => Ok(Command::Doctor),
        "publish" => args.get(1).map_or_else(
            || Err(CliError::MissingArgument("file")),
            |path| Ok(Command::Publish { path: path.clone() }),
        ),
        "record" => parse_record(&args[1..]),
        other => Err(CliError::UnknownCommand(other.to_string())),
    }
}

fn parse_record(args: &[String]) -> Result<Command, CliError> {
    let mut seconds: Option<u32> = None;
    let mut source: Option<String> = None;
    let mut out = String::from("vrecnu-recording.mp4");
    let mut fps: u32 = 30;
    let mut mic = false;
    let mut system = false;
    let mut camera: Option<String> = None;
    let mut bubble = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--seconds" => {
                let value = args
                    .get(index + 1)
                    .ok_or(CliError::MissingArgument("seconds"))?;
                seconds = Some(value.parse().map_err(|_| CliError::BadArgument {
                    flag: "seconds",
                    value: value.clone(),
                })?);
                index += 2;
            }
            "--source" => {
                source = Some(
                    args.get(index + 1)
                        .ok_or(CliError::MissingArgument("source"))?
                        .clone(),
                );
                index += 2;
            }
            "--out" => {
                out = args
                    .get(index + 1)
                    .ok_or(CliError::MissingArgument("out"))?
                    .clone();
                index += 2;
            }
            "--fps" => {
                let value = args
                    .get(index + 1)
                    .ok_or(CliError::MissingArgument("fps"))?;
                fps = value.parse().ok().filter(|n| *n > 0).ok_or_else(|| {
                    // Zero would panic the pacer, so it is rejected here rather than there.
                    CliError::BadArgument {
                        flag: "fps",
                        value: value.clone(),
                    }
                })?;
                index += 2;
            }
            "--mic" => {
                mic = true;
                index += 1;
            }
            "--system" => {
                system = true;
                index += 1;
            }
            // The one flag here whose value is optional. `--camera` alone means "the system
            // default"; a following token that starts with `--` is the next flag, not a device
            // id, so it is left for the loop rather than swallowed.
            "--camera" => {
                let value = args
                    .get(index + 1)
                    .filter(|next| !next.starts_with("--"))
                    .cloned();
                index += 1 + usize::from(value.is_some());
                camera = Some(value.unwrap_or_default());
            }
            "--bubble" => {
                bubble = true;
                index += 1;
            }
            other => return Err(CliError::UnknownCommand(other.to_string())),
        }
    }

    Ok(Command::Record(RecordOptions {
        seconds: seconds.ok_or(CliError::MissingArgument("seconds"))?,
        source,
        out,
        fps,
        mic,
        system,
        camera,
        bubble,
    }))
}

/// Parses and executes.
pub fn run(args: &[String]) -> Result<(), CliError> {
    match parse(args)? {
        Command::Help => {
            println!("{USAGE}");
            Ok(())
        }
        Command::Doctor => {
            doctor();
            Ok(())
        }
        Command::Sources => sources(),
        Command::Cameras => cameras(),
        Command::Record(options) => record(&options),
        Command::Publish { path } => publish(&path),
    }
}

/// Serves one file to the publish page and waits for a share link.
///
/// This is the whole Phase 2 publish path with no shell in it: bind a registered loopback
/// port, open the bundled page in the system browser, and wait for the page to POST a result
/// back over that same origin. The engine never sees an OAuth token or a byte of the upload
/// (AGENTS.md §5), and the fact that a *headless* binary can drive it is the ADR-006 proof --
/// if this needed a webview, the boundary would already be gone.
fn publish(path: &str) -> Result<(), CliError> {
    let session = Session::new(Arc::new(CliShell));
    let recording = session
        .adopt_recording(Path::new(path))
        .map_err(app_failure)?;
    let events = RecorderEngine::subscribe(&session);
    let handle = RecorderEngine::begin_publish(&session).map_err(app_failure)?;

    println!(
        "Serving {} ({:.1} MB) on port {}.",
        recording.path.display(),
        recording.size_bytes as f64 / (1024.0 * 1024.0),
        handle.port
    );
    println!(
        "
Open this in your browser if it does not open by itself:

  {}
",
        handle.url
    );
    if ShellServices::open_external_url(&CliShell, &handle.url).is_err() {
        println!("(vrecnu could not launch a browser for you.)");
    }
    println!("Waiting for the publish page. Ctrl-C to give up.");

    // Blocking on the event stream is right here and wrong in a GUI: this process has nothing
    // else to do, and the shell-api contract only forbids the *engine* from blocking a caller
    // that has to keep repainting.
    for event in &events {
        match event {
            EngineEvent::PublishProgress { bytes_sent, total } => {
                let percent = if total == 0 {
                    0.0
                } else {
                    (bytes_sent as f64 / total as f64) * 100.0
                };
                println!("  uploaded {percent:.0}%");
            }
            EngineEvent::PublishDone { web_view_link, .. } => {
                println!(
                    "
Done. Share link:

  {web_view_link}
"
                );
                return Ok(());
            }
            EngineEvent::PublishFailed { error } => {
                return Err(CliError::Engine(error.to_string()));
            }
            _ => {}
        }
    }
    // The sender is dropped when the pump exits, which without a terminal event means the
    // server timed out or was torn down.
    Err(CliError::Engine(
        "the publish session ended without a result".into(),
    ))
}

/// Flattens a boundary error into a CLI message, keeping the developer detail.
fn app_failure(app: vrecnu_shell_api::AppError) -> CliError {
    let mut text = app.to_string();
    if let Some(detail) = &app.detail {
        text.push_str(
            "
  detail: ",
        );
        text.push_str(detail);
    }
    CliError::Engine(text)
}

/// Flattens an engine failure into a CLI message, keeping the developer detail.
///
/// A GUI shows `message` and hides `detail` behind a disclosure triangle; a headless binary that
/// did the same would throw away the only copy of FFmpeg's stderr, which is the whole reason
/// this binary is useful for debugging M2-M5.
fn engine_failure(err: vrecnu_engine::EngineError) -> CliError {
    let app = vrecnu_shell_api::AppError::from(err);
    let mut text = app.to_string();
    if let Some(detail) = &app.detail {
        text.push_str("\n  detail: ");
        text.push_str(detail);
    }
    CliError::Engine(text)
}

fn sources() -> Result<(), CliError> {
    let sources = vrecnu_engine::list_capture_sources().map_err(engine_failure)?;
    if sources.is_empty() {
        println!("no capture sources found");
        return Ok(());
    }
    for source in sources {
        let marker = if source.is_primary { "*" } else { " " };
        // A minimized window's size is unknown until it is restored, and the engine reports it
        // as 0x0; print `-` rather than a size the recording would contradict.
        let size = if source.size.width == 0 {
            String::from("-")
        } else {
            format!("{}x{}", source.size.width, source.size.height)
        };
        println!(
            "{marker} {:<24} {:>10}  {:?}  {}",
            source.id.0, size, source.kind, source.title
        );
    }
    Ok(())
}

fn cameras() -> Result<(), CliError> {
    let cameras = vrecnu_engine::list_cameras().map_err(engine_failure)?;
    if cameras.is_empty() {
        println!("no cameras found");
        return Ok(());
    }
    for camera in cameras {
        println!("  {:<12} {}", camera.id, camera.name);
    }
    Ok(())
}

fn record(options: &RecordOptions) -> Result<(), CliError> {
    let shell = CliShell;
    let ffmpeg = shell
        .ffmpeg_path()
        .map_err(|err| CliError::Engine(err.to_string()))?;

    let request = RecordRequest {
        source: options.source.clone().map(CaptureSourceId),
        fps: options.fps,
        video_bitrate: 6_000_000,
        output: PathBuf::from(&options.out),
        ffmpeg,
        duration: Some(Duration::from_secs(u64::from(options.seconds))),
        mic: options.mic,
        system: options.system,
        camera: options.camera.clone(),
        // A bubble with no camera in it is an empty circle floating over the desktop, so the
        // window is only ever offered alongside a device.
        bubble: (options.bubble && options.camera.is_some()).then(BubbleSpec::default),
    };

    println!(
        "recording {} s at {} fps to {}{}",
        options.seconds,
        options.fps,
        options.out,
        match (options.mic, options.system) {
            (true, true) => " (mic + system audio)",
            (true, false) => " (mic)",
            (false, true) => " (system audio)",
            (false, false) => "",
        }
    );
    if let Some(device) = &options.camera {
        println!(
            "camera            {}{}",
            if device.is_empty() {
                "system default"
            } else {
                device
            },
            if options.bubble {
                " (floating bubble shown)"
            } else {
                " (burned in, no window)"
            }
        );
    }

    // Never set here: a fixed-duration run stops on its own frame budget. The flag exists so
    // the same call is what a shell with a Stop button will make.
    let stop = AtomicBool::new(false);
    let outcome = vrecnu_engine::record(&request, &stop, &mut |stats, elapsed| {
        println!(
            "  {:>5.1}s  in {}  written {}  duplicated {}  dropped {}  encoder {:.1} fps",
            elapsed.as_secs_f32(),
            stats.frames_in,
            stats.frames_written,
            stats.frames_duplicated,
            stats.frames_dropped,
            stats.encoder_fps
        );
    })
    .map_err(engine_failure)?;

    println!();
    println!("wrote      {}", outcome.output.display());
    println!("size       {}x{}", outcome.size.width, outcome.size.height);
    println!("elapsed    {:.2}s", outcome.elapsed.as_secs_f32());
    println!("frames in  {}", outcome.stats.frames_in);
    println!(
        "written    {} ({} duplicated)",
        outcome.stats.frames_written, outcome.stats.frames_duplicated
    );
    // The number the AGENTS.md §1 budget is written against, so it gets its own line rather
    // than being buried in a summary string.
    println!("dropped    {}", outcome.stats.frames_dropped);
    Ok(())
}

fn doctor() {
    println!("vrecnu-cli {}", env!("CARGO_PKG_VERSION"));
    println!();
    // Proving the engine is reachable from a shell with no GUI framework is the entire
    // purpose of this binary, so `doctor` exercises it rather than just printing versions.
    let machine = SessionMachine::new();
    println!(
        "engine            reachable (session state: {:?})",
        machine.state()
    );
    println!(
        "publish ports     {}",
        REGISTERED_PORTS
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    match vrecnu_engine::list_capture_sources() {
        Ok(sources) => println!("capture backend   ready ({} sources)", sources.len()),
        Err(err) => println!("capture backend   unavailable ({err})"),
    }
    let shell = CliShell;
    match shell.ffmpeg_path() {
        Ok(path) => {
            println!("ffmpeg            {}", path.display());
            // Probing rather than trusting the path: `h264_mf` is listed in every Windows build
            // but fails to open where Media Foundation has no encoder, and finding that out
            // mid-recording costs the recording.
            match vrecnu_engine::probe_encoder(&path) {
                Ok(encoder) => println!("h264 encoder      {encoder}"),
                Err(err) => println!("h264 encoder      unavailable ({err})"),
            }
        }
        Err(err) => println!("ffmpeg            {err}"),
    }
    let audio = vrecnu_engine::probe_audio();
    println!(
        "audio backend     mic {}, system {}",
        if audio.mic { "ready" } else { "unavailable" },
        if audio.system { "ready" } else { "unavailable" }
    );
    match vrecnu_engine::list_cameras() {
        Ok(cameras) if cameras.is_empty() => println!("camera backend    ready (no cameras found)"),
        Ok(cameras) => println!("camera backend    ready ({} cameras)", cameras.len()),
        Err(err) => println!("camera backend    unavailable ({err})"),
    }
    // A build made before `packages/publish` was built still compiles, but cannot publish;
    // say so here rather than at the end of someone's first recording.
    match PublishAssets::embedded() {
        Ok(assets) => println!("publish page      embedded ({} files)", assets.len()),
        Err(err) => println!(
            "publish page      missing: {err} (run `pnpm --filter @vrecnu/publish build`, then rebuild)"
        ),
    }
    println!();
    println!("This binary links the whole vrecnu engine with no GUI framework present,");
    println!("which is what keeps the shell swappable. See read_this_idiot.md.");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn no_arguments_shows_usage() {
        assert_eq!(parse(&[]).unwrap(), Command::Help);
    }

    #[test]
    fn record_requires_a_duration() {
        // Silently defaulting would make a mistyped flag look like a successful recording.
        assert!(matches!(
            parse(&argv(&["record"])),
            Err(CliError::MissingArgument("seconds"))
        ));
    }

    #[test]
    fn camera_without_an_id_means_the_system_default() {
        // Distinct from absent: `None` is "no camera", `Some("")` is "whichever one Windows
        // considers default". Collapsing them would make it impossible to record without one.
        let Command::Record(options) =
            parse(&argv(&["record", "--seconds", "5", "--camera"])).unwrap()
        else {
            panic!("expected a record command");
        };
        assert_eq!(options.camera, Some(String::new()));
    }

    #[test]
    fn camera_takes_the_following_token_as_a_device_id() {
        let Command::Record(options) =
            parse(&argv(&["record", "--seconds", "5", "--camera", "1"])).unwrap()
        else {
            panic!("expected a record command");
        };
        assert_eq!(options.camera, Some("1".into()));
    }

    #[test]
    fn a_flag_after_camera_is_not_swallowed_as_a_device_id() {
        // The trap in an optional-value flag: `--camera --bubble` must not open a camera named
        // "--bubble" and silently drop the bubble.
        let Command::Record(options) =
            parse(&argv(&["record", "--seconds", "5", "--camera", "--bubble"])).unwrap()
        else {
            panic!("expected a record command");
        };
        assert_eq!(options.camera, Some(String::new()));
        assert!(options.bubble);
    }

    #[test]
    fn cameras_is_its_own_command() {
        assert_eq!(parse(&argv(&["cameras"])).unwrap(), Command::Cameras);
    }

    #[test]
    fn record_parses_its_flags() {
        let parsed = parse(&argv(&[
            "record",
            "--seconds",
            "30",
            "--out",
            "clip.mp4",
            "--mic",
            "--system",
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            Command::Record(RecordOptions {
                seconds: 30,
                source: None,
                out: "clip.mp4".into(),
                fps: 30,
                mic: true,
                system: true,
                camera: None,
                bubble: false,
            })
        );
    }

    #[test]
    fn a_zero_frame_rate_is_rejected_at_the_boundary() {
        // The pacer asserts `fps > 0`; catching it here turns a panic into a usage error.
        assert!(matches!(
            parse(&argv(&["record", "--seconds", "5", "--fps", "0"])),
            Err(CliError::BadArgument { flag: "fps", .. })
        ));
    }

    #[test]
    fn record_options_carry_mic_and_system_through_to_the_request() {
        // `run()` itself is not exercised here — it would start a real recording, which needs
        // real capture/ffmpeg/audio hardware and belongs in a manual hardware check
        // (TESTING.md), not `cargo test`. This only proves parsing reaches `RecordOptions`
        // correctly, matching every other record-parsing test in this module.
        let Command::Record(options) =
            parse(&argv(&["record", "--seconds", "5", "--mic", "--system"])).unwrap()
        else {
            panic!("expected a record command");
        };
        assert!(options.mic);
        assert!(options.system);
    }

    #[test]
    fn record_defaults_the_output_path() {
        let Command::Record(options) = parse(&argv(&["record", "--seconds", "5"])).unwrap() else {
            panic!("expected a record command");
        };
        assert_eq!(options.out, "vrecnu-recording.mp4");
        assert!(!options.mic);
    }

    #[test]
    fn a_non_numeric_duration_is_rejected_by_name() {
        let err = parse(&argv(&["record", "--seconds", "soon"])).unwrap_err();
        assert!(matches!(
            err,
            CliError::BadArgument {
                flag: "seconds",
                ..
            }
        ));
        assert!(err.to_string().contains("soon"));
    }

    #[test]
    fn publish_requires_a_file() {
        assert!(matches!(
            parse(&argv(&["publish"])),
            Err(CliError::MissingArgument("file"))
        ));
    }

    #[test]
    fn an_unknown_command_points_at_help() {
        let err = parse(&argv(&["frobnicate"])).unwrap_err();
        assert!(err.to_string().contains("help"), "{err}");
    }

    #[test]
    fn an_unknown_record_flag_is_rejected_rather_than_ignored() {
        assert!(parse(&argv(&["record", "--seconds", "5", "--turbo"])).is_err());
    }

    #[test]
    fn publishing_a_file_that_is_not_there_fails_before_a_port_is_bound() {
        // Not just "it errors": it must error *without* having bound one of the five
        // registered ports, or a mistyped filename leaves the next publish reporting
        // `AllPortsBusy`. This test would hang, not fail, if the order were wrong -- the
        // command blocks on the event stream once a server is up.
        let err = run(&argv(&["publish", "definitely-not-a-real-file.mp4"])).unwrap_err();
        assert!(
            err.to_string().contains("could not be read"),
            "expected a read failure, got: {err}"
        );
    }

    #[test]
    fn doctor_runs_without_a_gui_framework_present() {
        assert!(run(&argv(&["doctor"])).is_ok());
    }
}
