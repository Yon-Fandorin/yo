use std::{
    sync::{atomic::Ordering, mpsc::TryRecvError},
    task::{Context, Poll},
};

use super::{
    AgentControlOutcome, AgentSession, AgentSessionError, WORKER_IDLE, WorkerSignal, WorkerTerminal,
};
use crate::{
    InputAdmissionConfigurationError, InputAdmissionHost, SubmissionOutcome, TranscriptReader,
};

/// live Session의 Journal과 worker에 대한 비차단 상태입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentSessionPoll {
    /// 현재 읽을 수 있는 Journal 변경이나 terminal worker 상태가 없습니다.
    Pending,
    /// [`TranscriptReader`]에서 하나 이상의 commit된 record를 읽을 수 있습니다.
    Changed,
    /// 앞선 모든 Journal 변경을 공개한 뒤 worker가 종료되었습니다.
    Closed,
}

impl AgentSession {
    /// 독립된 conversation을 만들 수 있는 현재 worker/input 상태를 반환합니다.
    #[must_use]
    pub fn is_idle_for_new_conversation(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
            && self.lifecycle.load(Ordering::Acquire) == WORKER_IDLE
            && !self.context_compaction_pending.load(Ordering::Acquire)
            && self.state.try_lock().is_ok_and(|state| {
                state.active_turn.is_none() && state.outstanding_requests.is_empty()
            })
    }

    /// live Session에서 입력이 queue되기 전에 실행 권한을 한 번 설정합니다.
    /// 재개된 history는 새 submission으로 세지 않습니다.
    pub fn configure_input_admission(
        &mut self,
        host: Box<dyn InputAdmissionHost>,
    ) -> Result<(), InputAdmissionConfigurationError> {
        if self.input_admission_sealed {
            return Err(InputAdmissionConfigurationError::InputAlreadySubmitted);
        }
        self.input_admission
            .set(host)
            .map_err(|_| InputAdmissionConfigurationError::AlreadyConfigured)
    }

    /// commit된 Journal history 또는 worker 상태가 바뀌었는지 관찰합니다.
    ///
    /// `Changed` 결과에는 semantic data가 없습니다. frontend는
    /// [`Self::transcript_reader`]로 commit된 suffix를 읽습니다.
    pub fn poll(&mut self) -> Result<AgentSessionPoll, AgentSessionError> {
        self.poll_with_observation(|| {})
    }

    #[cfg(test)]
    pub(super) fn poll_with_test_hook(
        &mut self,
        before_receive: impl FnOnce(),
    ) -> Result<AgentSessionPoll, AgentSessionError> {
        self.poll_with_observation(before_receive)
    }

    fn poll_with_observation(
        &mut self,
        before_receive: impl FnOnce(),
    ) -> Result<AgentSessionPoll, AgentSessionError> {
        let Some(changes) = self.changes.as_mut() else {
            return Ok(AgentSessionPoll::Closed);
        };
        let mut terminal = self
            .terminal
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        before_receive();
        match changes
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .try_recv()
        {
            Ok(WorkerSignal::Changed) => Ok(AgentSessionPoll::Changed),
            Err(TryRecvError::Empty) => Self::poll_terminal(&mut terminal, false),
            Err(TryRecvError::Disconnected) => Self::poll_terminal(&mut terminal, true),
        }
    }

    fn poll_terminal(
        terminal: &mut Option<WorkerTerminal>,
        disconnected: bool,
    ) -> Result<AgentSessionPoll, AgentSessionError> {
        match terminal.take() {
            Some(WorkerTerminal::Failure(error)) => Err(error),
            Some(WorkerTerminal::Closed) => Ok(AgentSessionPoll::Closed),
            None if disconnected => Ok(AgentSessionPoll::Closed),
            None => Ok(AgentSessionPoll::Pending),
        }
    }

    /// 다음 Session 변경을 관찰할 frontend task를 등록합니다.
    /// 소비되지 않은 worker signal은 `poll`이 소비할 때까지 반복 probe에서도 ready 상태입니다.
    pub fn poll_ready(&self, context: &mut Context<'_>) -> Poll<()> {
        let Some(changes) = self.changes.as_ref() else {
            return Poll::Ready(());
        };
        let terminal = self
            .terminal
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if terminal.is_some() {
            return Poll::Ready(());
        }
        changes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .poll_ready(context)
    }

    /// 이 Session의 commit된 semantic history에 read-only로 접근합니다.
    #[must_use]
    pub fn transcript_reader(&self) -> TranscriptReader {
        self.transcript.clone()
    }

    /// payload가 없는 Request correlation trace에 read-only로 접근합니다.
    #[must_use]
    pub fn request_trace_reader(&self) -> crate::RequestTraceReader {
        self.request_trace.clone()
    }

    /// 가장 오래된 whole-request admission 결과를 가져옵니다.
    pub fn take_submission_outcome(&mut self) -> Option<SubmissionOutcome> {
        self.submission_outcomes.lock().ok()?.pop_front()
    }

    /// 가장 오래된 완료된 control 결과를 가져옵니다.
    pub fn take_control_outcome(&mut self) -> Option<AgentControlOutcome> {
        self.control_outcomes.lock().ok()?.pop_front()
    }
}
