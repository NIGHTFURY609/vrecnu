//! Forwards engine events to the webview.
//!
//! The pump is shell-owned: engine threads never see an `AppHandle` (ADR-006's second leak
//! path). Event *contents* are never logged — `PublishOpened` carries the capability token.

use std::thread::JoinHandle;

use crossbeam_channel::Receiver;
use tauri::{AppHandle, Emitter};
use vrecnu_shell_api::EngineEvent;

/// The single Tauri event the UI listens to. Its payload is `EngineEvent` as serialized.
pub(crate) const ENGINE_EVENT: &str = "engine-event";

/// Where pumped events go. A trait so the pump is testable without a webview.
pub(crate) trait EventSink: Send + 'static {
    fn send(&self, event: &EngineEvent) -> std::result::Result<(), String>;
}

/// Emits to every webview of the app.
pub(crate) struct WebviewSink(AppHandle);

impl WebviewSink {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self(app)
    }
}

impl EventSink for WebviewSink {
    fn send(&self, event: &EngineEvent) -> std::result::Result<(), String> {
        self.0
            .emit(ENGINE_EVENT, event)
            .map_err(|err| err.to_string())
    }
}

/// Drains `events` into `sink` until the engine drops its sender.
pub(crate) fn spawn_pump(
    events: Receiver<EngineEvent>,
    sink: impl EventSink,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("vrecnu-event-pump".into())
        .spawn(move || {
            for event in events {
                if let Err(err) = sink.send(&event) {
                    tracing::warn!(%err, "an engine event did not reach the window");
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Records what reached the "webview"; fails the sends listed in `fail_on`.
    #[derive(Clone, Default)]
    struct FakeSink {
        seen: Arc<Mutex<Vec<EngineEvent>>>,
        fail_on: Vec<usize>,
        calls: Arc<Mutex<usize>>,
    }

    impl EventSink for FakeSink {
        fn send(&self, event: &EngineEvent) -> std::result::Result<(), String> {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            if self.fail_on.contains(&*calls) {
                return Err("no window".into());
            }
            self.seen.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    #[test]
    fn events_reach_the_sink_in_order_and_the_pump_ends_with_the_engine() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let sink = FakeSink::default();
        let pump = spawn_pump(rx, sink.clone()).unwrap();
        tx.send(EngineEvent::Elapsed { ms: 1 }).unwrap();
        tx.send(EngineEvent::Elapsed { ms: 2 }).unwrap();
        drop(tx);
        pump.join().unwrap();
        assert_eq!(
            *sink.seen.lock().unwrap(),
            vec![
                EngineEvent::Elapsed { ms: 1 },
                EngineEvent::Elapsed { ms: 2 }
            ]
        );
    }

    #[test]
    fn a_failed_send_does_not_stop_the_pump() {
        // No window open (or one closing) must not cost the next window its events.
        let (tx, rx) = crossbeam_channel::unbounded();
        let sink = FakeSink {
            fail_on: vec![1],
            ..FakeSink::default()
        };
        let pump = spawn_pump(rx, sink.clone()).unwrap();
        tx.send(EngineEvent::Elapsed { ms: 1 }).unwrap();
        tx.send(EngineEvent::Elapsed { ms: 2 }).unwrap();
        drop(tx);
        pump.join().unwrap();
        assert_eq!(
            *sink.seen.lock().unwrap(),
            vec![EngineEvent::Elapsed { ms: 2 }]
        );
    }
}
