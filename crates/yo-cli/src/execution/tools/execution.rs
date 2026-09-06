use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread::{self, JoinHandle},
};

use yo_core::{
    ToolExecution, ToolExecutionError, ToolExecutionOutcome, ToolExecutionPoll, ToolExecutionResult,
};

pub(super) fn completed(output: String, truncated: bool) -> ToolExecutionResult {
    ToolExecutionResult::new(ToolExecutionOutcome::Completed, output, truncated)
}

pub(super) fn failed(message: &str) -> ToolExecutionResult {
    ToolExecutionResult::new(ToolExecutionOutcome::Failed, message, false)
}

pub(super) fn interrupted() -> ToolExecutionResult {
    ToolExecutionResult::new(ToolExecutionOutcome::Interrupted, "interrupted", false)
}

pub(super) struct ThreadExecution {
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<ToolExecutionResult>,
    worker: Option<JoinHandle<()>>,
    result: Option<ToolExecutionResult>,
}

impl ThreadExecution {
    pub(super) fn spawn(
        task: impl FnOnce(Arc<AtomicBool>) -> ToolExecutionResult + Send + 'static,
    ) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let _ = sender.send(task(worker_cancelled));
        });
        Self {
            cancelled,
            receiver,
            worker: Some(worker),
            result: None,
        }
    }

    fn poll_result(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError> {
        if self.result.is_some() {
            return Ok(ToolExecutionPoll::Ready);
        }
        match self.receiver.try_recv() {
            Ok(result) => {
                self.result = Some(result);
                Ok(ToolExecutionPoll::Ready)
            },
            Err(TryRecvError::Empty) => Ok(ToolExecutionPoll::Pending),
            Err(TryRecvError::Disconnected) => {
                Err(ToolExecutionError::new("local tool worker stopped"))
            },
        }
    }

    pub(super) fn join(&mut self) -> Result<(), ToolExecutionError> {
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| ToolExecutionError::new("local tool worker panicked"))?;
        }
        Ok(())
    }
}

impl ToolExecution for ThreadExecution {
    fn poll(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError> {
        self.poll_result()
    }

    fn take_result(&mut self) -> Option<ToolExecutionResult> {
        self.result.take()
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        self.cancel();
        self.join()
    }
}
