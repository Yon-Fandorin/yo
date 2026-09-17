use std::{sync::atomic::Ordering, thread::JoinHandle};

use super::{AgentSession, AgentSessionError, WORKER_IDLE};
use crate::{
    JournalDurability,
    session_repository::{
        SessionForkLimits, StoredSessionContinuation, StoredSessionForkCatalog,
        StoredSessionForkSelection, StoredSessionReader, read_fork_catalog,
        read_stored_session_continuation,
    },
};

impl AgentSession {
    /// 다른 parent writer lease를 획득하지 않고 현재 durable fork source를 캡처합니다.
    /// live 작업이 대기 중이거나 storage가 live durable 경계와 다르면 실패합니다.
    pub fn capture_fork_source(
        &self,
        reader: &(impl StoredSessionReader + ?Sized),
    ) -> Result<StoredSessionContinuation, AgentSessionError> {
        let before = self.inspect_fork_parent()?;
        let captured = read_stored_session_continuation(reader, self.session_id)
            .map_err(|error| AgentSessionError::WorkerUnavailable(error.to_string()))?;
        let after = self.inspect_fork_parent()?;
        if before != after || captured.journal_cutoff() != Some(before.1) {
            return Err(AgentSessionError::WorkerUnavailable(
                "fork source differs from the live durable boundary".to_owned(),
            ));
        }
        Ok(captured)
    }

    /// idle live parent에서 물리적으로 제한되고 완전히 검증된 historical catalog를 캡처합니다.
    pub fn capture_fork_catalog(
        &self,
        reader: &(impl StoredSessionReader + ?Sized),
        limits: SessionForkLimits,
    ) -> Result<StoredSessionForkCatalog, AgentSessionError> {
        let before = self.inspect_fork_parent()?;
        let catalog = read_fork_catalog(reader, self.session_id, limits)
            .map_err(|error| AgentSessionError::WorkerUnavailable(error.to_string()))?;
        if before != self.inspect_fork_parent()? || catalog.durability() != before.0 {
            return Err(AgentSessionError::WorkerUnavailable(
                "historical fork capture differs from the live durable boundary".to_owned(),
            ));
        }
        Ok(catalog)
    }

    /// 최신 live parent를 다시 검증한 뒤 opaque selected point만 재구성합니다.
    /// 새 작업, compaction, binding 변경, durability 손실이 있으면 selection을 다시 만듭니다.
    pub fn prepare_historical_fork_source(
        &self,
        selection: &StoredSessionForkSelection,
    ) -> Result<StoredSessionContinuation, AgentSessionError> {
        let before = self.inspect_fork_parent()?;
        let source = selection
            .prepare_source(self.session_id, before.0)
            .map_err(|error| AgentSessionError::WorkerUnavailable(error.to_string()))?;
        if before != self.inspect_fork_parent()? {
            return Err(AgentSessionError::WorkerUnavailable(
                "historical fork selection became stale during preparation".to_owned(),
            ));
        }
        Ok(source)
    }

    fn inspect_fork_parent(
        &self,
    ) -> Result<(JournalDurability, crate::JournalSequence), AgentSessionError> {
        if self.worker.as_ref().is_none_or(JoinHandle::is_finished)
            || self.lifecycle.load(Ordering::Acquire) != WORKER_IDLE
            || self.context_compaction_pending.load(Ordering::Acquire)
        {
            return Err(AgentSessionError::WorkerUnavailable(
                "fork capture requires an idle live Session".to_owned(),
            ));
        }
        let state = self.state.try_lock().map_err(|_| {
            AgentSessionError::WorkerUnavailable(
                "fork capture cannot overtake a Session state update".to_owned(),
            )
        })?;
        if state.active_turn.is_some() || !state.outstanding_requests.is_empty() {
            return Err(AgentSessionError::WorkerUnavailable(
                "fork capture cannot overtake pending input or requests".to_owned(),
            ));
        }
        let durability = self.transcript.durability();
        match durability {
            JournalDurability::Durable {
                journal_sequence: Some(cutoff),
                ..
            } => Ok((durability, cutoff)),
            _ => Err(AgentSessionError::WorkerUnavailable(
                "fork capture requires committed durable history".to_owned(),
            )),
        }
    }
}
