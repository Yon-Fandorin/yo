use std::{fmt, sync::mpsc, thread};

use tokio::sync::mpsc as async_mpsc;
use yo_core::{
    ConnectorError, ConnectorFailureKind, ModelConnectorCancellation, ModelConnectorEvent,
    ModelConnectorPoll,
};

use super::failure::transport_failure;

pub struct ConnectorStream {
    pub(crate) receiver: async_mpsc::Receiver<ModelConnectorEvent>,
    pub(crate) outcome: mpsc::Receiver<Result<(), ConnectorError>>,
    pub(crate) cancellation: ModelConnectorCancellation,
    pub(crate) worker: Option<thread::JoinHandle<()>>,
    pub(crate) closed: bool,
}

impl ConnectorStream {
    pub fn poll(&mut self) -> Result<ModelConnectorPoll, ConnectorError> {
        if self.closed {
            return Ok(ModelConnectorPoll::Closed);
        }
        match self.receiver.try_recv() {
            Ok(event) => return Ok(ModelConnectorPoll::Event(event)),
            Err(async_mpsc::error::TryRecvError::Empty) => return Ok(ModelConnectorPoll::Pending),
            Err(async_mpsc::error::TryRecvError::Disconnected) => {},
        }
        match self.outcome.try_recv() {
            Ok(Ok(())) => {
                self.closed = true;
                Ok(ModelConnectorPoll::Closed)
            },
            Ok(Err(error)) => {
                self.closed = true;
                Err(error)
            },
            Err(mpsc::TryRecvError::Empty) => Ok(ModelConnectorPoll::Pending),
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

impl Drop for ConnectorStream {
    fn drop(&mut self) {
        self.cancellation.cancel();
        let _ = self.join_worker();
    }
}

impl fmt::Debug for ConnectorStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectorStream")
            .field("cancelled", &self.cancellation.is_cancelled())
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}
