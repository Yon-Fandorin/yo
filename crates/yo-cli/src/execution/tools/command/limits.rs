use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(super) const OUTPUT_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const PROCESS_CLEANUP_GRACE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
pub(super) struct CommandExecutionLimits {
    pub(super) output_inactivity_timeout: Duration,
    pub(super) absolute_execution_timeout: Option<Duration>,
    pub(super) cleanup_grace: Duration,
}

impl CommandExecutionLimits {
    pub(super) fn for_agent(absolute_execution_timeout: Option<Duration>) -> Self {
        Self {
            output_inactivity_timeout: OUTPUT_INACTIVITY_TIMEOUT,
            absolute_execution_timeout,
            cleanup_grace: PROCESS_CLEANUP_GRACE,
        }
    }

    pub(super) fn is_valid(self) -> bool {
        !self.output_inactivity_timeout.is_zero()
            && !self.cleanup_grace.is_zero()
            && !self
                .absolute_execution_timeout
                .is_some_and(|timeout| timeout.is_zero())
    }
}

#[derive(Clone, Copy)]
pub(super) enum StopReason {
    Cancelled,
    OutputInactivity,
    AbsoluteDeadline,
}

impl StopReason {
    pub(super) const fn description(self) -> &'static str {
        match self {
            Self::Cancelled => "run_command cancelled",
            Self::OutputInactivity => "run_command output inactivity deadline expired",
            Self::AbsoluteDeadline => "run_command absolute execution deadline expired",
        }
    }
}

pub(super) fn expired_reason(
    cancelled: &AtomicBool,
    attempt_started: Instant,
    last_output_progress: Instant,
    limits: CommandExecutionLimits,
    observed_at: Instant,
) -> Option<StopReason> {
    if cancelled.load(Ordering::Acquire) {
        return Some(StopReason::Cancelled);
    }
    let inactivity_expired = observed_at.saturating_duration_since(last_output_progress)
        >= limits.output_inactivity_timeout;
    let absolute_expired = limits
        .absolute_execution_timeout
        .is_some_and(|timeout| observed_at.saturating_duration_since(attempt_started) >= timeout);
    match (inactivity_expired, absolute_expired) {
        (false, false) => None,
        (true, false) => Some(StopReason::OutputInactivity),
        (false, true) => Some(StopReason::AbsoluteDeadline),
        (true, true) => {
            let inactivity_deadline = last_output_progress
                .checked_add(limits.output_inactivity_timeout)
                .unwrap_or(observed_at);
            let absolute_deadline = limits
                .absolute_execution_timeout
                .and_then(|timeout| attempt_started.checked_add(timeout))
                .unwrap_or(observed_at);
            Some(if absolute_deadline <= inactivity_deadline {
                StopReason::AbsoluteDeadline
            } else {
                StopReason::OutputInactivity
            })
        },
    }
}
