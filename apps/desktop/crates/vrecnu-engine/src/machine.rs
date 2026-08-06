use vrecnu_types::SessionState;

/// An action that was not legal in the current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cannot {action} while {state:?}")]
pub struct TransitionError {
    /// State the machine was in.
    pub state: SessionState,
    /// Action that was attempted.
    pub action: &'static str,
}

/// The session lifecycle.
///
/// Mirrors `recorderReducer` in `packages/core` deliberately, so the desktop app and the PWA
/// describe a recording's life the same way and a bug found in one is findable in the other.
///
/// One rule carried across from the web side, where getting it wrong stranded the UI with no
/// way out: **`fail()` is legal from every non-terminal state.** Anything that can fail must
/// be able to say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionMachine {
    state: SessionState,
}

impl Default for SessionMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionMachine {
    /// A machine at rest.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: SessionState::Idle,
        }
    }

    /// Current state.
    #[must_use]
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// Devices opened and permissions proven.
    pub fn arm(&mut self) -> Result<(), TransitionError> {
        match self.state {
            // Re-arming after a failure is how the user retries without a separate reset.
            SessionState::Idle | SessionState::Stopped | SessionState::Failed => {
                self.state = SessionState::Ready;
                Ok(())
            }
            state => Err(TransitionError {
                state,
                action: "arm",
            }),
        }
    }

    /// Capture begins.
    pub fn start(&mut self) -> Result<(), TransitionError> {
        match self.state {
            SessionState::Ready => {
                self.state = SessionState::Recording;
                Ok(())
            }
            state => Err(TransitionError {
                state,
                action: "start recording",
            }),
        }
    }

    /// Capture ends; the encoder is still draining.
    pub fn stop(&mut self) -> Result<(), TransitionError> {
        match self.state {
            SessionState::Recording => {
                self.state = SessionState::Finalizing;
                Ok(())
            }
            state => Err(TransitionError {
                state,
                action: "stop recording",
            }),
        }
    }

    /// The encoder finished and a file exists.
    pub fn finalized(&mut self) -> Result<(), TransitionError> {
        match self.state {
            SessionState::Finalizing => {
                self.state = SessionState::Stopped;
                Ok(())
            }
            state => Err(TransitionError {
                state,
                action: "finalize",
            }),
        }
    }

    /// Something failed. Always legal except when already failed.
    pub fn fail(&mut self) {
        self.state = SessionState::Failed;
    }

    /// Back to rest.
    pub fn reset(&mut self) {
        self.state = SessionState::Idle;
    }

    /// Whether capture is currently running.
    #[must_use]
    pub const fn is_recording(&self) -> bool {
        matches!(self.state, SessionState::Recording)
    }

    /// Whether a finished recording is available to publish or export.
    #[must_use]
    pub const fn has_recording(&self) -> bool {
        matches!(self.state, SessionState::Stopped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_happy_path_runs_end_to_end() {
        let mut machine = SessionMachine::new();
        assert_eq!(machine.state(), SessionState::Idle);
        machine.arm().unwrap();
        assert_eq!(machine.state(), SessionState::Ready);
        machine.start().unwrap();
        assert!(machine.is_recording());
        machine.stop().unwrap();
        assert_eq!(machine.state(), SessionState::Finalizing);
        machine.finalized().unwrap();
        assert!(machine.has_recording());
        machine.reset();
        assert_eq!(machine.state(), SessionState::Idle);
    }

    #[test]
    fn failure_is_reachable_from_every_non_terminal_state() {
        // The web-side bug this mirrors: `ERROR` was rejected from `requesting-permissions`,
        // so a denied prompt stranded the UI in a busy state with no way out.
        for setup in [
            SessionState::Idle,
            SessionState::Ready,
            SessionState::Recording,
            SessionState::Finalizing,
            SessionState::Stopped,
        ] {
            let mut machine = SessionMachine::new();
            machine.state = setup;
            machine.fail();
            assert_eq!(machine.state(), SessionState::Failed, "from {setup:?}");
        }
    }

    #[test]
    fn a_user_can_retry_straight_from_failure() {
        let mut machine = SessionMachine::new();
        machine.fail();
        machine.arm().expect("re-arming after a failure must work");
        assert_eq!(machine.state(), SessionState::Ready);
    }

    #[test]
    fn recording_cannot_start_before_arming() {
        let mut machine = SessionMachine::new();
        let err = machine.start().unwrap_err();
        assert_eq!(err.state, SessionState::Idle);
        assert_eq!(
            machine.state(),
            SessionState::Idle,
            "a rejected action must not move the machine"
        );
    }

    #[test]
    fn stopping_requires_an_active_recording() {
        let mut machine = SessionMachine::new();
        machine.arm().unwrap();
        assert!(machine.stop().is_err());
    }

    #[test]
    fn finalizing_twice_is_rejected() {
        let mut machine = SessionMachine::new();
        machine.arm().unwrap();
        machine.start().unwrap();
        machine.stop().unwrap();
        machine.finalized().unwrap();
        assert!(machine.finalized().is_err());
    }

    #[test]
    fn a_transition_error_names_the_action_it_refused() {
        let mut machine = SessionMachine::new();
        let err = machine.stop().unwrap_err();
        assert!(err.to_string().contains("stop recording"), "{err}");
    }
}
