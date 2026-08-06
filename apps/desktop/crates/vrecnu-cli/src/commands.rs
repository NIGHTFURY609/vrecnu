//! Argument parsing and command dispatch.
//!
//! Hand-rolled rather than `clap`: four subcommands do not justify a dependency, and this
//! crate's dependency graph is load-bearing evidence rather than a convenience.

use std::fmt;

use vrecnu_engine::SessionMachine;
use vrecnu_publish_server::REGISTERED_PORTS;

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
    record --seconds <N>        Record for N seconds, then encode
        [--source <ID>]         Which screen or window (default: primary display)
        [--out <PATH>]          Output file (default: vrecnu-recording.mp4)
        [--mic] [--system]      Include microphone / system audio
    publish <FILE>              Serve FILE to the publish page and wait for a share link
    doctor                      Report what this build can do on this machine
    help                        Show this message

ENVIRONMENT:
    VRECNU_LOG                  Log filter, e.g. `debug` or `vrecnu_capture=trace`

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
    /// Include the microphone.
    pub mic: bool,
    /// Include system audio.
    pub system: bool,
}

/// A parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// List capture sources.
    Sources,
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
    let mut mic = false;
    let mut system = false;

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
            "--mic" => {
                mic = true;
                index += 1;
            }
            "--system" => {
                system = true;
                index += 1;
            }
            other => return Err(CliError::UnknownCommand(other.to_string())),
        }
    }

    Ok(Command::Record(RecordOptions {
        seconds: seconds.ok_or(CliError::MissingArgument("seconds"))?,
        source,
        out,
        mic,
        system,
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
        // Capture, encode and publish arrive in M2-M5. Reporting that plainly beats a stub
        // that pretends to record and writes an empty file.
        Command::Sources => Err(CliError::Engine(
            "listing capture sources needs the capture backend, which lands in M2".into(),
        )),
        Command::Record(_) => Err(CliError::Engine(
            "recording needs the capture and encode pipeline, which lands in M2".into(),
        )),
        Command::Publish { .. } => Err(CliError::Engine(
            "publishing needs the loopback server, which lands in M5".into(),
        )),
    }
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
    println!("capture backend   not yet implemented (M2)");
    println!("audio backend     not yet implemented (M3)");
    println!("camera + bubble   not yet implemented (M1/M4)");
    println!("publish server    not yet implemented (M5)");
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
                mic: true,
                system: true,
            })
        );
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
    fn unimplemented_commands_say_so_instead_of_pretending() {
        // A stub that "succeeds" and writes nothing is worse than an honest refusal.
        let err = run(&argv(&["record", "--seconds", "1"])).unwrap_err();
        assert!(err.to_string().contains("M2"), "{err}");
    }

    #[test]
    fn doctor_runs_without_a_gui_framework_present() {
        assert!(run(&argv(&["doctor"])).is_ok());
    }
}
