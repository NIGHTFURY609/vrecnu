//! Pins the JSON the webview receives. Writes packages/desktop-ui/src/__fixtures__/engine.json
//! with VRECNU_UPDATE_FIXTURES=1; otherwise fails if serialization drifted from it. The vitest
//! side (contract.test.ts) fails if engine.ts drifted from the same file.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Serialize;
use vrecnu_shell_api::{AppError, AppState, EngineEvent};
use vrecnu_types::{
    AudioDevice, AudioDevices, CameraInfo, CaptureSource, CaptureSourceId, CaptureSourceKind,
    CaptureStats, ErrorCode, Recording, RenderId, SessionId, SessionState, Size,
};

const UPDATE_ENV: &str = "VRECNU_UPDATE_FIXTURES";

/// Field order is fixed by the struct, not by serde_json's map (whose ordering depends on
/// workspace feature unification).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    events: Vec<EngineEvent>,
    app_state: AppState,
    sources: Vec<CaptureSource>,
    cameras: Vec<CameraInfo>,
    audio_devices: AudioDevices,
}

const ID: SessionId = SessionId(1_760_000_000_000);

fn error() -> AppError {
    AppError::recoverable(
        ErrorCode::PublishPortsBusy,
        "All five publish ports are in use.",
    )
    .with_hint("Close whatever is using ports 47821-47825 and try again.")
    .with_detail("bind failed")
}

fn recording() -> Recording {
    Recording {
        session: ID,
        path: PathBuf::from("C:/recordings/vrecnu-1760000000000.mp4"),
        duration_ms: 10_000,
        size_bytes: 4_096,
        mime_type: "video/mp4".into(),
    }
}

fn stats() -> CaptureStats {
    // Exactly representable floats, so the fixture is stable across platforms.
    CaptureStats {
        frames_in: 300,
        frames_written: 300,
        frames_dropped: 0,
        frames_duplicated: 12,
        encoder_fps: 29.5,
    }
}

fn events() -> Vec<EngineEvent> {
    vec![
        EngineEvent::StateChanged {
            from: SessionState::Ready,
            to: SessionState::Recording,
        },
        EngineEvent::Elapsed { ms: 1_500 },
        EngineEvent::AudioLevels {
            mic_db: -12.5,
            system_db: -30.25,
        },
        EngineEvent::Stats(stats()),
        EngineEvent::Warning {
            code: ErrorCode::CaptureBackpressure,
            message: "Frames are being dropped.".into(),
        },
        EngineEvent::RenderProgress {
            id: RenderId(7),
            fraction: 0.25,
        },
        EngineEvent::RenderDone {
            id: RenderId(7),
            recording: recording(),
        },
        EngineEvent::PublishOpened {
            url: "http://127.0.0.1:47821/?t=TOKEN".into(),
            port: 47_821,
        },
        EngineEvent::PublishProgress {
            bytes_sent: 1_024,
            total: 4_096,
        },
        EngineEvent::PublishDone {
            web_view_link: "https://drive.google.com/file/d/FILE_ID/view".into(),
            file_id: "FILE_ID".into(),
        },
        EngineEvent::PublishFailed { error: error() },
        EngineEvent::Failed { error: error() },
    ]
}

/// No wildcard arm: a new variant fails to compile here until the fixture covers it.
fn variant(event: &EngineEvent) -> &'static str {
    match event {
        EngineEvent::StateChanged { .. } => "stateChanged",
        EngineEvent::Elapsed { .. } => "elapsed",
        EngineEvent::AudioLevels { .. } => "audioLevels",
        EngineEvent::Stats(_) => "stats",
        EngineEvent::Warning { .. } => "warning",
        EngineEvent::RenderProgress { .. } => "renderProgress",
        EngineEvent::RenderDone { .. } => "renderDone",
        EngineEvent::PublishOpened { .. } => "publishOpened",
        EngineEvent::PublishProgress { .. } => "publishProgress",
        EngineEvent::PublishDone { .. } => "publishDone",
        EngineEvent::PublishFailed { .. } => "publishFailed",
        EngineEvent::Failed { .. } => "failed",
    }
}

fn fixture() -> Fixture {
    Fixture {
        events: events(),
        app_state: AppState {
            session: SessionState::Stopped,
            session_id: Some(ID),
            elapsed_ms: 10_000,
            stats: stats(),
            recording: Some(recording()),
            error: None,
            publishing: false,
        },
        sources: vec![CaptureSource {
            id: CaptureSourceId("monitor:1".into()),
            title: "Display 1".into(),
            kind: CaptureSourceKind::Monitor,
            size: Size {
                width: 1920,
                height: 1080,
            },
            is_primary: true,
        }],
        cameras: vec![CameraInfo {
            id: "0".into(),
            name: "USB Camera".into(),
        }],
        audio_devices: AudioDevices {
            inputs: vec![AudioDevice {
                id: "in".into(),
                name: "Microphone".into(),
                is_default: true,
            }],
            outputs: vec![AudioDevice {
                id: "out".into(),
                name: "Speakers".into(),
                is_default: true,
            }],
        },
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../packages/desktop-ui/src/__fixtures__/engine.json")
}

#[test]
fn the_fixture_has_one_event_of_every_variant() {
    let names: BTreeSet<_> = events().iter().map(variant).collect();
    assert_eq!(names.len(), events().len());
}

#[test]
fn the_desktop_ui_fixture_matches_what_the_engine_serializes() {
    let rendered = serde_json::to_string_pretty(&fixture()).unwrap() + "\n";
    let path = fixture_path();
    if std::env::var_os(UPDATE_ENV).is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &rendered).unwrap();
        return;
    }
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{}: {err}; regenerate with {UPDATE_ENV}=1", path.display()))
        .replace("\r\n", "\n");
    assert_eq!(
        on_disk, rendered,
        "the engine's JSON changed: regenerate with {UPDATE_ENV}=1 and update packages/desktop-ui/src/engine.ts"
    );
}
