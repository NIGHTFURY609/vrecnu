//! What the tray shows, and when the window comes and goes, as the engine moves.
//!
//! Driven only by engine events, so a recording started from the window and one started from
//! the tray menu behave the same. The decisions are pure functions; the observer thread applies
//! them through [`TraySurface`], so none of this needs a tray or a webview to test.

use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use vrecnu_shell_api::EngineEvent;
use vrecnu_types::SessionState;

/// The tray item that starts a recording.
pub(crate) const START_LABEL: &str = "Start recording";
/// The same item while a recording runs.
pub(crate) const STOP_LABEL: &str = "Stop recording";
const IDLE_TOOLTIP: &str = "vrecnu";

/// Everything the tray displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrayView {
    pub(crate) tooltip: String,
    pub(crate) record_label: &'static str,
    pub(crate) record_enabled: bool,
}

/// What happens to the window on a state change (spec D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowAction {
    /// Create the window, or focus it if it exists.
    Open,
    /// Destroy it: a recording must not capture vrecnu, and no renderer runs while recording.
    Close,
}

/// What the tray's Start/Stop item does in a given state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordAction {
    Start,
    Stop,
    Nothing,
}

/// Where the tray observer sends its decisions. A trait so the observer is testable.
pub(crate) trait TraySurface: Send + 'static {
    fn show(&self, view: &TrayView);
    fn window(&self, action: WindowAction);
}

pub(crate) fn tray_view(state: SessionState, elapsed_ms: u64) -> TrayView {
    match state {
        SessionState::Recording => TrayView {
            tooltip: format!("vrecnu - recording {}", format_elapsed(elapsed_ms)),
            record_label: STOP_LABEL,
            record_enabled: true,
        },
        // Disabled rather than relabelled: Stop has been pressed, and Start would be refused
        // until FFmpeg finishes the file.
        SessionState::Finalizing => TrayView {
            tooltip: "vrecnu - saving the recording".to_owned(),
            record_label: STOP_LABEL,
            record_enabled: false,
        },
        SessionState::Idle | SessionState::Ready | SessionState::Stopped | SessionState::Failed => {
            TrayView {
                tooltip: IDLE_TOOLTIP.to_owned(),
                record_label: START_LABEL,
                record_enabled: true,
            }
        }
    }
}

/// `m:ss`, or `h:mm:ss` from an hour up. The window's `formatElapsed` matches it.
pub(crate) fn format_elapsed(ms: u64) -> String {
    let total = ms / 1000;
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

pub(crate) fn window_action(from: SessionState, to: SessionState) -> Option<WindowAction> {
    match (from, to) {
        (_, SessionState::Recording) => Some(WindowAction::Close),
        // Back to the user with the result, or with why there is none.
        (
            SessionState::Recording | SessionState::Finalizing,
            SessionState::Stopped | SessionState::Failed,
        ) => Some(WindowAction::Open),
        _ => None,
    }
}

pub(crate) fn record_action(state: SessionState) -> RecordAction {
    match state {
        SessionState::Recording => RecordAction::Stop,
        SessionState::Finalizing => RecordAction::Nothing,
        SessionState::Idle | SessionState::Ready | SessionState::Stopped | SessionState::Failed => {
            RecordAction::Start
        }
    }
}

/// Whether FFmpeg may still be writing the file.
pub(crate) fn is_capturing(state: SessionState) -> bool {
    matches!(state, SessionState::Recording | SessionState::Finalizing)
}

/// Polls `done` until it holds or `timeout` passes. Returns whether it held.
pub(crate) fn wait_until(
    timeout: Duration,
    poll: Duration,
    mut done: impl FnMut() -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(poll);
    }
}

/// Follows the engine until it drops its sender, keeping the tray and the window in step.
pub(crate) fn spawn_observer(
    events: Receiver<EngineEvent>,
    initial: SessionState,
    surface: impl TraySurface,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("vrecnu-tray".into())
        .spawn(move || {
            let mut state = initial;
            let mut elapsed_ms = 0;
            let mut shown = tray_view(state, elapsed_ms);
            surface.show(&shown);
            for event in events {
                match event {
                    EngineEvent::StateChanged { from, to } => {
                        state = to;
                        if to == SessionState::Recording {
                            elapsed_ms = 0;
                        }
                        if let Some(action) = window_action(from, to) {
                            surface.window(action);
                        }
                    }
                    EngineEvent::Elapsed { ms } => elapsed_ms = ms,
                    _ => continue,
                }
                // `Elapsed` arrives at ~4 Hz and the tooltip changes once a second.
                let view = tray_view(state, elapsed_ms);
                if view != shown {
                    surface.show(&view);
                    shown = view;
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use vrecnu_types::SessionState::{Failed, Finalizing, Idle, Ready, Recording, Stopped};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Show(TrayView),
        Window(WindowAction),
    }

    #[derive(Clone, Default)]
    struct FakeSurface(Arc<Mutex<Vec<Call>>>);

    impl TraySurface for FakeSurface {
        fn show(&self, view: &TrayView) {
            self.0.lock().unwrap().push(Call::Show(view.clone()));
        }
        fn window(&self, action: WindowAction) {
            self.0.lock().unwrap().push(Call::Window(action));
        }
    }

    fn run(initial: SessionState, events: Vec<EngineEvent>) -> Vec<Call> {
        let (tx, rx) = crossbeam_channel::unbounded();
        let surface = FakeSurface::default();
        let observer = spawn_observer(rx, initial, surface.clone()).unwrap();
        for event in events {
            tx.send(event).unwrap();
        }
        drop(tx);
        observer.join().unwrap();
        let calls = surface.0.lock().unwrap().clone();
        calls
    }

    fn changed(from: SessionState, to: SessionState) -> EngineEvent {
        EngineEvent::StateChanged { from, to }
    }

    fn tooltips(calls: &[Call]) -> Vec<&str> {
        calls
            .iter()
            .filter_map(|call| match call {
                Call::Show(view) => Some(view.tooltip.as_str()),
                Call::Window(_) => None,
            })
            .collect()
    }

    fn windows(calls: &[Call]) -> Vec<WindowAction> {
        calls
            .iter()
            .filter_map(|call| match call {
                Call::Window(action) => Some(*action),
                Call::Show(_) => None,
            })
            .collect()
    }

    #[test]
    fn the_tooltip_counts_up_while_recording_and_settles_after() {
        let calls = run(
            Idle,
            vec![
                changed(Idle, Ready),
                changed(Ready, Recording),
                EngineEvent::Elapsed { ms: 65_000 },
                changed(Recording, Finalizing),
                changed(Finalizing, Stopped),
            ],
        );
        assert_eq!(
            tooltips(&calls),
            vec![
                "vrecnu",
                "vrecnu - recording 0:00",
                "vrecnu - recording 1:05",
                "vrecnu - saving the recording",
                "vrecnu",
            ]
        );
    }

    #[test]
    fn ticks_within_the_same_second_do_not_repaint_the_tray() {
        let calls = run(
            Recording,
            vec![
                EngineEvent::Elapsed { ms: 1_000 },
                EngineEvent::Elapsed { ms: 1_250 },
                EngineEvent::Elapsed { ms: 1_750 },
                EngineEvent::Elapsed { ms: 2_000 },
            ],
        );
        assert_eq!(
            tooltips(&calls),
            vec![
                "vrecnu - recording 0:00",
                "vrecnu - recording 0:01",
                "vrecnu - recording 0:02",
            ]
        );
    }

    #[test]
    fn a_new_recording_counts_from_zero_not_from_the_last_one() {
        let calls = run(
            Recording,
            vec![
                EngineEvent::Elapsed { ms: 9_000 },
                changed(Recording, Finalizing),
                changed(Finalizing, Stopped),
                changed(Stopped, Ready),
                changed(Ready, Recording),
            ],
        );
        assert_eq!(tooltips(&calls).last(), Some(&"vrecnu - recording 0:00"));
    }

    #[test]
    fn starting_closes_the_window_and_finishing_reopens_it() {
        let calls = run(
            Idle,
            vec![
                changed(Idle, Ready),
                changed(Ready, Recording),
                changed(Recording, Finalizing),
                changed(Finalizing, Stopped),
            ],
        );
        assert_eq!(
            windows(&calls),
            vec![WindowAction::Close, WindowAction::Open]
        );
    }

    #[test]
    fn a_failed_recording_reopens_the_window_to_say_why() {
        assert_eq!(window_action(Recording, Failed), Some(WindowAction::Open));
        assert_eq!(window_action(Finalizing, Failed), Some(WindowAction::Open));
    }

    #[test]
    fn changes_outside_a_recording_leave_the_window_alone() {
        // Reset from the window, or arming, must not close or reopen it under the user.
        for (from, to) in [
            (Stopped, Idle),
            (Failed, Idle),
            (Idle, Ready),
            (Idle, Failed),
        ] {
            assert_eq!(window_action(from, to), None, "{from:?} -> {to:?}");
        }
    }

    #[test]
    fn the_record_item_stops_while_recording_and_is_disabled_while_saving() {
        let recording = tray_view(Recording, 0);
        assert_eq!(
            (recording.record_label, recording.record_enabled),
            (STOP_LABEL, true)
        );
        let saving = tray_view(Finalizing, 0);
        assert_eq!(
            (saving.record_label, saving.record_enabled),
            (STOP_LABEL, false)
        );
        for state in [Idle, Ready, Stopped, Failed] {
            let view = tray_view(state, 0);
            assert_eq!(
                (view.record_label, view.record_enabled),
                (START_LABEL, true)
            );
        }
    }

    #[test]
    fn the_record_item_does_what_its_label_says() {
        assert_eq!(record_action(Recording), RecordAction::Stop);
        assert_eq!(record_action(Finalizing), RecordAction::Nothing);
        for state in [Idle, Ready, Stopped, Failed] {
            assert_eq!(record_action(state), RecordAction::Start);
        }
    }

    #[test]
    fn elapsed_time_reads_as_minutes_then_hours() {
        assert_eq!(format_elapsed(0), "0:00");
        assert_eq!(format_elapsed(59_999), "0:59");
        assert_eq!(format_elapsed(61_000), "1:01");
        assert_eq!(format_elapsed(3_600_000), "1:00:00");
        assert_eq!(format_elapsed(3_725_000), "1:02:05");
    }

    #[test]
    fn waiting_returns_as_soon_as_the_condition_holds() {
        let mut polls = 0;
        let held = wait_until(Duration::from_secs(5), Duration::from_millis(1), || {
            polls += 1;
            polls >= 3
        });
        assert!(held);
        assert_eq!(polls, 3);
    }

    #[test]
    fn waiting_gives_up_at_the_deadline() {
        // Quit must still quit if FFmpeg never finishes the file.
        let started = Instant::now();
        assert!(!wait_until(
            Duration::from_millis(50),
            Duration::from_millis(5),
            || false
        ));
        assert!(started.elapsed() >= Duration::from_millis(50));
    }
}
