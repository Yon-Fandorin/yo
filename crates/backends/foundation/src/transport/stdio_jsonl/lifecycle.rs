use std::{
    sync::{Arc, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

use super::peer::{JsonlPoll, ProcessControl, StdioJsonlPeer};
use crate::{BackendFailure, BackendFailureKind, BackendStopHandle};

impl ProcessControl {
    pub(super) fn request_stop(&self) {
        self.stop_requested.store(true, Ordering::Release);
        let Ok(mut child) = self.child.lock() else {
            return;
        };
        if matches!(child.try_wait(), Ok(None)) {
            let _ = child.kill();
        }
    }
}

impl StdioJsonlPeer {
    pub fn stop_handle(&self) -> BackendStopHandle {
        let process = Arc::clone(&self.process);
        BackendStopHandle::new(move || process.request_stop())
    }

    pub fn shutdown(&mut self) -> Result<(), BackendFailure> {
        if let Some(result) = &self.shutdown_result {
            return result.clone();
        }
        self.stdin.take();
        // shutdown_once drains and joins stderr before enriching the failure. This
        // retains diagnostics even when the first stdin write raced an early exit.
        let result = self
            .shutdown_once()
            .map_err(|failure| self.with_failure_diagnostic(failure));
        self.shutdown_result = Some(result.clone());
        result
    }

    pub(super) fn closed_poll(&self) -> Result<JsonlPoll, BackendFailure> {
        if self.shutdown_result.is_some() {
            return Ok(JsonlPoll::Closed);
        }
        let detail = self.stderr_detail();
        let message = detail.map_or_else(
            || format!("{} exited unexpectedly", self.process_name),
            |detail| format!("{} exited unexpectedly: {detail}", self.process_name),
        );
        Err(BackendFailure::new(
            BackendFailureKind::ProcessExit,
            message,
        ))
    }

    fn stderr_detail(&self) -> Option<String> {
        let tail = self.stderr_tail.lock().ok()?;
        match self.stderr_diagnostic {
            Some(diagnostic) => diagnostic(&tail).map(str::to_owned),
            None => (!tail.trim().is_empty()).then(|| tail.trim().to_owned()),
        }
    }

    pub(super) fn with_failure_diagnostic(&self, failure: BackendFailure) -> BackendFailure {
        // Preserve the behavior of adapters that have not opted into safe diagnostics.
        if self.stderr_diagnostic.is_none() {
            return failure;
        }
        match self.stderr_detail() {
            Some(detail) => {
                BackendFailure::new(failure.kind(), format!("{}; {detail}", failure.message()))
            },
            None => failure,
        }
    }

    fn shutdown_once(&mut self) -> Result<(), BackendFailure> {
        let deadline = Instant::now() + self.shutdown_timeout;
        let process_failure = loop {
            let status = self
                .process
                .child
                .lock()
                .map_err(|_| cleanup_failure(self.process_name, "process lock was poisoned"))?
                .try_wait();
            match status {
                Ok(Some(status)) if status.success() => break None,
                Ok(Some(_)) if self.process.stop_requested.load(Ordering::Acquire) => break None,
                Ok(Some(status)) => {
                    break Some(cleanup_failure(
                        self.process_name,
                        format!("exited with {status}"),
                    ));
                },
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                Ok(None) => {
                    let mut child = self.process.child.lock().map_err(|_| {
                        cleanup_failure(self.process_name, "process lock was poisoned")
                    })?;
                    child.kill().map_err(|error| {
                        cleanup_failure(self.process_name, format!("failed to terminate: {error}"))
                    })?;
                    child.wait().map_err(|error| {
                        cleanup_failure(self.process_name, format!("failed to reap: {error}"))
                    })?;
                    break None;
                },
                Err(error) => {
                    return Err(cleanup_failure(
                        self.process_name,
                        format!("failed to inspect process: {error}"),
                    ));
                },
            }
        };
        let reader_failure = self.drain_and_join().err();
        match (process_failure, reader_failure) {
            (None, None) => Ok(()),
            (Some(failure), None) | (None, Some(failure)) => Err(failure),
            (Some(process), Some(reader)) => Err(cleanup_failure(
                self.process_name,
                format!("{process}; additionally, {reader}"),
            )),
        }
    }

    fn drain_and_join(&mut self) -> Result<(), BackendFailure> {
        if let Some(reader) = self.reader.take() {
            while !reader.is_finished() {
                while self.receiver.try_recv().is_ok() {}
                thread::yield_now();
            }
            while self.receiver.try_recv().is_ok() {}
            reader
                .join()
                .map_err(|_| cleanup_failure(self.process_name, "stdout reader panicked"))?;
        }
        if let Some(reader) = self.stderr_reader.take() {
            reader
                .join()
                .map_err(|_| cleanup_failure(self.process_name, "stderr reader panicked"))?;
        }
        Ok(())
    }
}

impl Drop for StdioJsonlPeer {
    fn drop(&mut self) {
        if self.shutdown_result.is_none() {
            self.process.request_stop();
            let _ = self.shutdown();
        }
    }
}

fn cleanup_failure(process_name: &str, message: impl Into<String>) -> BackendFailure {
    BackendFailure::new(
        BackendFailureKind::Cleanup,
        format!("{process_name}: {}", message.into()),
    )
}
