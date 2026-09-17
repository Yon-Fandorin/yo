use std::sync::mpsc::{self, SyncSender, TrySendError};

use super::{AgentSession, AgentSessionError, WORKER_IDLE, WORKER_POLL_INTERVAL};
use crate::{AgentBackend, BackendFailure};

pub(super) struct ReplacementRequest {
    pub(super) backend: Box<dyn AgentBackend + Send>,
    pub(super) result: SyncSender<Result<BackendReplacementOutcome, AgentSessionError>>,
}

fn reject_replacement_after_cleanup(
    backend: &mut (dyn AgentBackend + Send),
    primary: AgentSessionError,
) -> AgentSessionError {
    match backend.shutdown() {
        Ok(()) => primary,
        Err(cleanup) => AgentSessionError::Multiple {
            primary: Box::new(primary),
            additional: Box::new(AgentSessionError::BackendCleanup(cleanup)),
        },
    }
}

/// idle backend를 원자적으로 교체한 결과입니다.
#[derive(Clone, Debug)]
pub struct BackendReplacementOutcome {
    pub(super) cleanup_failure: Option<BackendFailure>,
}

impl BackendReplacementOutcome {
    /// 새 epoch가 commit된 뒤 이전 backend 정리에서 발생한 오류를 반환합니다.
    #[must_use]
    pub const fn cleanup_failure(&self) -> Option<&BackendFailure> {
        self.cleanup_failure.as_ref()
    }
}

impl AgentSession {
    /// idle exact-replay Session의 backend를 원자적으로 교체합니다.
    ///
    /// 후보 resume 또는 durable transition publication이 실패해도 현재 backend는 유지됩니다.
    /// 취소는 후보만 멈추고 worker가 해당 결정을 완료할 때까지 기다립니다.
    pub fn replace_backend(
        &mut self,
        mut backend: Box<dyn AgentBackend + Send>,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<BackendReplacementOutcome, AgentSessionError> {
        if self
            .context_compaction_pending
            .load(std::sync::atomic::Ordering::Acquire)
        {
            let primary = AgentSessionError::WorkerUnavailable(
                "binding replacement cannot overtake pending context compaction".to_owned(),
            );
            return Err(reject_replacement_after_cleanup(&mut *backend, primary));
        }
        if self.lifecycle.load(std::sync::atomic::Ordering::Acquire) != WORKER_IDLE {
            let primary = AgentSessionError::WorkerUnavailable(
                "binding replacement requires an idle Session".to_owned(),
            );
            return Err(reject_replacement_after_cleanup(&mut *backend, primary));
        }
        let candidate_stop = backend.stop_handle();
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        if let Err(error) = self.replacements.try_send(ReplacementRequest {
            backend,
            result: result_tx,
        }) {
            let mut request = match error {
                TrySendError::Full(request) | TrySendError::Disconnected(request) => request,
            };
            let primary = AgentSessionError::WorkerUnavailable(
                "the backend replacement lane is unavailable".to_owned(),
            );
            return Err(reject_replacement_after_cleanup(
                &mut *request.backend,
                primary,
            ));
        }
        self.readiness.notify();
        let mut cancellation_requested = false;
        loop {
            match result_rx.recv_timeout(WORKER_POLL_INTERVAL) {
                Ok(result) => {
                    let outcome = result?;
                    self.stop = candidate_stop;
                    return Ok(outcome);
                },
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(AgentSessionError::WorkerUnavailable(
                        "the runtime worker closed during backend replacement".to_owned(),
                    ));
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if is_cancelled() && !cancellation_requested {
                        cancellation_requested = true;
                        candidate_stop.request_stop();
                    }
                },
            }
        }
    }
}
