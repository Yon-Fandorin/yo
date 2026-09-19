use std::{
    sync::{atomic::Ordering, mpsc::RecvTimeoutError},
    thread::JoinHandle,
};
#[cfg(test)]
use std::{
    thread,
    time::{Duration, Instant},
};

use super::{
    AgentSession, AgentSessionError, WORKER_GRACEFUL_SHUTDOWN, WORKER_SHUTDOWN_TIMEOUT,
    WORKER_STOPPING, WorkerExit, WorkerTerminal,
};
use crate::AgentEvent;

impl AgentSession {
    /// worker를 멈추고 backend를 정리한 뒤 terminal event를 반환합니다.
    pub fn shutdown(&mut self) -> Result<Vec<AgentEvent>, AgentSessionError> {
        let Some(worker) = self.worker.take() else {
            return Ok(Vec::new());
        };
        self.lifecycle.store(WORKER_STOPPING, Ordering::Release);
        let finished_gracefully = match self.finished.recv_timeout(WORKER_GRACEFUL_SHUTDOWN) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => true,
            Err(RecvTimeoutError::Timeout) => false,
        };
        if !finished_gracefully {
            self.stop.request_stop();
        }

        let mut failures = Vec::new();
        if let Some(changes) = self.changes.take() {
            drop(changes);
        }

        if !finished_gracefully && self.finished.recv_timeout(WORKER_SHUTDOWN_TIMEOUT).is_err() {
            if let Some(failure) = self.take_terminal_failure() {
                failures.push(failure);
            }
            failures.push(AgentSessionError::WorkerShutdownTimedOut);
            return Err(combine_failures(failures).expect("the shutdown timeout added one failure"));
        }
        match join_worker(worker) {
            Ok(exit) => {
                if let Some(failure) = self.take_terminal_failure() {
                    failures.push(failure);
                }
                if let Some(failure) = exit.failure {
                    failures.push(failure);
                }
                if let Some(error) = combine_failures(failures) {
                    Err(error)
                } else {
                    Ok(exit.terminal_events)
                }
            },
            Err(error) => {
                if let Some(failure) = self.take_terminal_failure() {
                    failures.push(failure);
                }
                failures.push(error);
                Err(combine_failures(failures).expect("join added one failure"))
            },
        }
    }

    fn take_terminal_failure(&self) -> Option<AgentSessionError> {
        let mut terminal = self
            .terminal
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if matches!(terminal.as_ref(), Some(WorkerTerminal::Failure(_))) {
            match terminal.take() {
                Some(WorkerTerminal::Failure(error)) => Some(error),
                Some(WorkerTerminal::Closed) | None => None,
            }
        } else {
            None
        }
    }

    #[cfg(test)]
    pub(super) fn wait_until_processed(&self, expected: u64) {
        let (processed, changed) = &*self.processed;
        let count = processed.lock().unwrap();
        let (count, timeout) = changed
            .wait_timeout_while(count, Duration::from_secs(5), |count| *count < expected)
            .unwrap();
        assert!(
            !timeout.timed_out(),
            "worker did not process {expected} actions"
        );
        assert!(*count >= expected);
    }

    #[cfg(test)]
    pub(super) fn wait_until_no_active_turn(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let inactive = self
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .active_turn
                .is_none();
            if inactive {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "worker did not apply the Turn completion"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Drop for AgentSession {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

pub(super) fn join_worker(worker: JoinHandle<WorkerExit>) -> Result<WorkerExit, AgentSessionError> {
    worker.join().map_err(|_| AgentSessionError::WorkerPanicked)
}

fn combine_failures(mut failures: Vec<AgentSessionError>) -> Option<AgentSessionError> {
    let mut combined = failures.pop()?;
    while let Some(primary) = failures.pop() {
        combined = AgentSessionError::Multiple {
            primary: Box::new(primary),
            additional: Box::new(combined),
        };
    }
    Some(combined)
}
