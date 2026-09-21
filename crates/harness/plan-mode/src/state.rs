//! Plan mode state machine and command parsing.
//!
//! Reference: `deepseek-harness-rs/src/plan_mode.rs` (Rust reference implementation)
//! and `dsh packages/plan/plan-mode` (behavioral spec).

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Maximum plan content size (16 KiB).
pub const MAX_PLAN_BYTES: usize = 16 * 1024;
/// Maximum command message size (1 KiB).
pub const MAX_PLAN_COMMAND_MESSAGE_BYTES: usize = 1_000;

/// Serializable plan mode change event.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanModeChange {
    active: bool,
}

impl PlanModeChange {
    /// Create a new plan mode change event.
    #[must_use]
    pub fn new(active: bool) -> Self {
        Self { active }
    }

    /// Get the active state.
    #[must_use]
    pub fn active(&self) -> bool {
        self.active
    }
}

/// Parsed plan mode command from user input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanModeCommand {
    /// Enter plan mode, optionally with an initial message.
    Enter { message: Option<String> },
    /// Exit plan mode directly.
    Off,
}

impl PlanModeCommand {
    /// Parse a user command string into a `PlanModeCommand`.
    ///
    /// Returns `None` if the input doesn't start with `/plan` (exact prefix).
    /// Returns `Some(Err(...))` if the command exceeds size limits.
    #[must_use]
    pub fn parse(input: &str) -> Option<Result<Self, PlanModeError>> {
        let input = input.trim_start();
        if !input.starts_with("/plan") {
            return None;
        }
        let rest = &input[5..];
        if rest.is_empty() {
            return Some(Ok(Self::Enter { message: None }));
        }
        let rest = rest.trim_start();
        if rest == "off" {
            return Some(Ok(Self::Off));
        }
        if rest.len() > MAX_PLAN_COMMAND_MESSAGE_BYTES {
            return Some(Err(PlanModeError::MessageTooLarge));
        }
        Some(Ok(Self::Enter {
            message: Some(rest.to_owned()),
        }))
    }
}

/// Errors that can occur during plan mode operations.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PlanModeError {
    /// Plan mode state is unavailable (mutex poisoned).
    #[error("Plan Mode state is unavailable")]
    Unavailable,
    /// Command message exceeds the interactive input limit.
    #[error("Plan Mode command message exceeds the interactive input limit")]
    MessageTooLarge,
    /// `exit_plan_mode` is only available in Plan Mode.
    #[error("exit_plan_mode is only available in Plan Mode")]
    Inactive,
    /// Plan mode changed before the prepared transition was committed.
    #[error("Plan Mode changed before the prepared transition was committed")]
    Stale,
}

/// Internal state of the plan mode runtime.
#[derive(Debug)]
struct PlanModeState {
    active: bool,
    pending: Option<bool>,
}

/// Runtime for managing plan mode state.
///
/// `Clone` + `Send` + `Sync`; wraps an `Arc<Mutex<PlanModeState>>`.
#[derive(Clone)]
pub struct PlanModeRuntime {
    state: Arc<Mutex<PlanModeState>>,
}

impl std::fmt::Debug for PlanModeRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanModeRuntime").finish()
    }
}

impl PlanModeRuntime {
    /// Create a new runtime with the given initial active state.
    #[must_use]
    pub fn new(active: bool) -> Self {
        Self {
            state: Arc::new(Mutex::new(PlanModeState {
                active,
                pending: None,
            })),
        }
    }

    /// Get the current active state.
    ///
    /// Returns `Err(PlanModeError::Unavailable)` if the mutex is poisoned.
    pub fn active(&self) -> Result<bool, PlanModeError> {
        self.state
            .lock()
            .map(|state| state.active)
            .map_err(|_| PlanModeError::Unavailable)
    }

    /// Parse a user command string.
    ///
    /// Returns `None` if the input doesn't match `/plan` prefix.
    /// Returns `Some(Ok(command))` or `Some(Err(error))` for valid prefixes.
    #[must_use]
    pub fn parse_command(&self, input: &str) -> Option<Result<PlanModeCommand, PlanModeError>> {
        PlanModeCommand::parse(input)
    }

    /// Prepare a mode change (enter/exit plan mode).
    ///
    /// Returns `Ok(Some(mutation))` if a change is needed.
    /// Returns `Ok(None)` if the target state equals the current state (idempotent).
    /// Returns `Err(PlanModeError::Stale)` if a pending change already exists.
    /// Returns `Err(PlanModeError::Unavailable)` if the mutex is poisoned.
    pub fn prepare_set(
        &self,
        active: bool,
    ) -> Result<Option<PreparedPlanModeMutation>, PlanModeError> {
        let state = self.state.lock().map_err(|_| PlanModeError::Unavailable)?;
        if state.pending.is_some() {
            return Err(PlanModeError::Stale);
        }
        if state.active == active {
            return Ok(None);
        }
        Ok(Some(PreparedPlanModeMutation {
            runtime: self.clone(),
            expected_active: state.active,
            target: active,
            boundary: false,
        }))
    }

    /// Prepare a boundary commit for a pending change.
    ///
    /// Returns `Ok(Some(mutation))` if there is a pending change to commit.
    /// Returns `Ok(None)` if no pending change exists.
    /// Returns `Err(PlanModeError::Stale)` if the pending target equals current (shouldn't happen).
    /// Returns `Err(PlanModeError::Unavailable)` if the mutex is poisoned.
    pub fn prepare_boundary(&self) -> Result<Option<PreparedPlanModeMutation>, PlanModeError> {
        let state = self.state.lock().map_err(|_| PlanModeError::Unavailable)?;
        let Some(target) = state.pending else {
            return Ok(None);
        };
        if target == state.active {
            return Err(PlanModeError::Stale);
        }
        Ok(Some(PreparedPlanModeMutation {
            runtime: self.clone(),
            expected_active: state.active,
            target,
            boundary: true,
        }))
    }

    /// Prepare an approved exit from plan mode.
    ///
    /// Only valid when currently active and no pending change exists.
    /// Returns `Err(PlanModeError::Inactive)` if not active.
    /// Returns `Err(PlanModeError::Stale)` if a pending change exists.
    /// Returns `Err(PlanModeError::Unavailable)` if the mutex is poisoned.
    pub fn prepare_approved_exit(&self) -> Result<PreparedPlanExit, PlanModeError> {
        let state = self.state.lock().map_err(|_| PlanModeError::Unavailable)?;
        if !state.active {
            return Err(PlanModeError::Inactive);
        }
        if state.pending.is_some() {
            return Err(PlanModeError::Stale);
        }
        Ok(PreparedPlanExit {
            runtime: self.clone(),
        })
    }

    /// Get the plan policy section for prompt injection.
    ///
    /// Returns the configured section when active, empty string when inactive.
    pub fn plan_policy_section(&self, section: &str) -> String {
        match self.active() {
            Ok(true) => section.to_string(),
            _ => String::new(),
        }
    }
}

/// A prepared plan mode mutation that must be committed to take effect.
///
/// Dropping without `commit()` rolls back the pending change.
#[derive(Debug)]
pub struct PreparedPlanModeMutation {
    runtime: PlanModeRuntime,
    expected_active: bool,
    target: bool,
    boundary: bool,
}

impl PreparedPlanModeMutation {
    /// Get the `PlanModeChange` this mutation would produce.
    #[must_use]
    pub const fn change(&self) -> PlanModeChange {
        PlanModeChange {
            active: self.target,
        }
    }

    /// Commit the mutation, making the state change permanent.
    ///
    /// Returns `Err(PlanModeError::Stale)` if the state changed unexpectedly.
    /// Returns `Err(PlanModeError::Unavailable)` if the mutex is poisoned.
    pub fn commit(self) -> Result<(), PlanModeError> {
        let mut state = self
            .runtime
            .state
            .lock()
            .map_err(|_| PlanModeError::Unavailable)?;
        let pending_matches = if self.boundary {
            state.pending == Some(self.target)
        } else {
            state.pending.is_none()
        };
        if state.active != self.expected_active || !pending_matches {
            return Err(PlanModeError::Stale);
        }
        state.active = self.target;
        if self.boundary {
            state.pending = None;
        }
        Ok(())
    }
}

/// A prepared plan exit that commits to `pending = false` on success.
#[derive(Debug)]
pub struct PreparedPlanExit {
    runtime: PlanModeRuntime,
}

impl PreparedPlanExit {
    /// Commit the approved exit, setting pending to inactive.
    ///
    /// Returns `Err(PlanModeError::Stale)` if not active or pending exists.
    /// Returns `Err(PlanModeError::Unavailable)` if the mutex is poisoned.
    pub fn commit(self) -> Result<(), PlanModeError> {
        let mut state = self
            .runtime
            .state
            .lock()
            .map_err(|_| PlanModeError::Unavailable)?;
        if !state.active || state.pending.is_some() {
            return Err(PlanModeError::Stale);
        }
        state.pending = Some(false);
        Ok(())
    }
}
