use std::{fmt, sync::mpsc, thread};

use tokio::sync::mpsc as async_mpsc;

use super::{
    super::{ConnectorError, ConnectorFailureKind, ResponsesEvent, ResponsesPoll},
    ResponsesCancellation,
    failure::transport_failure,
};

pub struct ResponsesStream {
    pub(super) receiver: async_mpsc::Receiver<ResponsesEvent>,
    pub(super) outcome: mpsc::Receiver<Result<(), ConnectorError>>,
    pub(super) cancellation: ResponsesCancellation,
    pub(super) worker: Option<thread::JoinHandle<()>>,
    pub(super) closed: bool,
}

impl ResponsesStream {
    pub fn poll(&mut self) -> Result<ResponsesPoll, ConnectorError> {
        if self.closed {
            return Ok(ResponsesPoll::Closed);
        }
        match self.receiver.try_recv() {
            Ok(event) => return Ok(ResponsesPoll::Event(event)),
            Err(async_mpsc::error::TryRecvError::Empty) => return Ok(ResponsesPoll::Pending),
            Err(async_mpsc::error::TryRecvError::Disconnected) => {},
        }
        match self.outcome.try_recv() {
            Ok(Ok(())) => {
                self.closed = true;
                Ok(ResponsesPoll::Closed)
            },
            Ok(Err(error)) => {
                self.closed = true;
                Err(error)
            },
            Err(mpsc::TryRecvError::Empty) => Ok(ResponsesPoll::Pending),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.closed = true;
                Err(transport_failure(
                    "model-connector request worker stopped without a terminal outcome",
                ))
            },
        }
    }

    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub fn shutdown(&mut self) -> Result<(), ConnectorError> {
        self.cancellation.cancel();
        self.closed = true;
        self.join_worker()
    }

    fn join_worker(&mut self) -> Result<(), ConnectorError> {
        self.worker.take().map_or(Ok(()), |worker| {
            worker.join().map_err(|_| {
                ConnectorError::new(
                    ConnectorFailureKind::Cleanup,
                    "model-connector request worker panicked during cleanup",
                )
            })
        })
    }
}

impl Drop for ResponsesStream {
    fn drop(&mut self) {
        self.cancellation.cancel();
        let _ = self.join_worker();
    }
}

impl fmt::Debug for ResponsesStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResponsesStream")
            .field("cancelled", &self.cancellation.is_cancelled())
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}
