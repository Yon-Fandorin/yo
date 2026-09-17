use std::sync::{
    Arc, Mutex,
    mpsc::{SyncSender, TrySendError},
};

use super::super::AgentSessionError;
use crate::{AgentEvent, readiness::Readiness};

pub(in crate::agent_session) enum WorkerSignal {
    Changed,
    Failure(AgentSessionError),
    Closed,
}

pub(in crate::agent_session) struct WorkerExit {
    pub(in crate::agent_session) terminal_events: Vec<AgentEvent>,
    pub(in crate::agent_session) failure: Option<AgentSessionError>,
}

impl WorkerExit {
    pub(in crate::agent_session) fn success() -> Self {
        Self {
            terminal_events: Vec::new(),
            failure: None,
        }
    }

    pub(in crate::agent_session) fn from_cleanup(
        result: Result<Vec<AgentEvent>, crate::RuntimeError>,
    ) -> Self {
        match result {
            Ok(terminal_events) => Self {
                terminal_events,
                failure: None,
            },
            Err(error) => Self {
                terminal_events: Vec::new(),
                failure: Some(AgentSessionError::Runtime(error)),
            },
        }
    }
}

pub(in crate::agent_session) struct ChangeLane {
    sender: SyncSender<WorkerSignal>,
    failure: Arc<Mutex<Option<AgentSessionError>>>,
    readiness: Arc<Readiness>,
}

impl ChangeLane {
    pub(in crate::agent_session) fn new(
        sender: SyncSender<WorkerSignal>,
        failure: Arc<Mutex<Option<AgentSessionError>>>,
        readiness: Arc<Readiness>,
    ) -> Self {
        Self {
            sender,
            failure,
            readiness,
        }
    }

    /// level-triggered wake-up을 공개합니다. 읽지 않은 알림 하나가
    /// frontend가 아직 소비하지 않은 모든 commit suffix를 나타냅니다.
    pub(in crate::agent_session) fn changed(&mut self) -> bool {
        let open = match self.sender.try_send(WorkerSignal::Changed) {
            Ok(()) | Err(TrySendError::Full(WorkerSignal::Changed)) => true,
            Err(TrySendError::Disconnected(_)) => false,
            Err(TrySendError::Full(_)) => {
                unreachable!("a terminal worker signal cannot precede another change")
            },
        };
        if open {
            self.readiness.notify();
        }
        open
    }

    pub(in crate::agent_session) fn failure(&mut self, error: AgentSessionError) -> bool {
        if let Ok(mut failure) = self.failure.lock() {
            *failure = Some(error.clone());
        }
        let open = self.sender.send(WorkerSignal::Failure(error)).is_ok();
        if open {
            self.readiness.notify();
        }
        open
    }

    pub(in crate::agent_session) fn close(&mut self) -> bool {
        let open = self.sender.send(WorkerSignal::Closed).is_ok();
        if open {
            self.readiness.notify();
        }
        open
    }
}
