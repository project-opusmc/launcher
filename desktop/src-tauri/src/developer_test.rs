use super::DeveloperTestProfile;
use std::sync::{Mutex, MutexGuard};

/// In-memory only state for exercising launcher UI lifecycle states during
/// local development. It deliberately has no dependency on authentication,
/// the runtime, the filesystem, or process spawning.
#[derive(Default)]
pub(super) struct DeveloperTestCoordinator {
    state: Mutex<DeveloperTestState>,
}

#[derive(Default)]
struct DeveloperTestState {
    active: bool,
    active_session: Option<String>,
    next_session: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SimulatedGameSession {
    pub session_id: String,
}

impl DeveloperTestCoordinator {
    fn lock_state(&self) -> MutexGuard<'_, DeveloperTestState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn profile(&self) -> DeveloperTestProfile {
        let state = self.lock_state();
        DeveloperTestProfile {
            available: true,
            active: state.active,
            simulation_active: state.active_session.is_some(),
        }
    }

    pub(super) fn set_active(&self, active: bool) -> Result<DeveloperTestProfile, String> {
        let mut state = self.lock_state();
        if !active && state.active_session.is_some() {
            return Err(
                "Finish the simulated game session before leaving Developer Test Profile"
                    .to_owned(),
            );
        }
        state.active = active;
        Ok(DeveloperTestProfile {
            available: true,
            active: state.active,
            simulation_active: state.active_session.is_some(),
        })
    }

    pub(super) fn start_simulation(&self) -> Result<SimulatedGameSession, String> {
        let mut state = self.lock_state();
        if !state.active {
            return Err("Developer Test Profile is not active".to_owned());
        }
        if state.active_session.is_some() {
            return Err("A simulated game session is already running".to_owned());
        }
        let session_id = format!("developer-test-{:04}", state.next_session);
        state.next_session = state.next_session.wrapping_add(1);
        state.active_session = Some(session_id.clone());
        Ok(SimulatedGameSession { session_id })
    }

    /// Marks the matching simulation as complete. A stale timer must not end a
    /// newer test session.
    pub(super) fn finish_simulation(&self, session_id: &str) -> Option<SimulatedGameSession> {
        let mut state = self.lock_state();
        if state.active_session.as_deref() != Some(session_id) {
            return None;
        }
        state.active_session = None;
        Some(SimulatedGameSession {
            session_id: session_id.to_owned(),
        })
    }
}
