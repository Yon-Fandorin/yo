use std::sync::{
    Arc, Mutex, PoisonError,
    mpsc::{SyncSender, TrySendError},
};

use super::super::AgentSessionError;
use crate::{AgentEvent, readiness::Readiness};

pub(in crate::agent_session) enum WorkerSignal {
    Changed,
}

pub(in crate::agent_session) enum WorkerTerminal {
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
    terminal: Arc<Mutex<Option<WorkerTerminal>>>,
    readiness: Arc<Readiness>,
}

impl ChangeLane {
    #[cfg(test)]
    pub(in crate::agent_session) fn new(
        sender: SyncSender<WorkerSignal>,
        readiness: Arc<Readiness>,
    ) -> Self {
        Self::with_terminal(sender, Arc::new(Mutex::new(None)), readiness)
    }

    pub(in crate::agent_session) fn with_terminal(
        sender: SyncSender<WorkerSignal>,
        terminal: Arc<Mutex<Option<WorkerTerminal>>>,
        readiness: Arc<Readiness>,
    ) -> Self {
        Self {
            sender,
            terminal,
            readiness,
        }
    }

    /// level-triggered wake-up을 공개합니다. 읽지 않은 알림 하나가
    /// frontend가 아직 소비하지 않은 모든 commit suffix를 나타냅니다.
    pub(in crate::agent_session) fn changed(&mut self) -> bool {
        let open = {
            // 같은 worker 전이의 Changed가 terminal 관찰 뒤로 넘어가지 않도록 직렬화
            let _terminal = self.terminal.lock().unwrap_or_else(PoisonError::into_inner);
            match self.sender.try_send(WorkerSignal::Changed) {
                Ok(()) | Err(TrySendError::Full(WorkerSignal::Changed)) => true,
                Err(TrySendError::Disconnected(_)) => false,
            }
        };
        if open {
            self.readiness.notify();
        }
        open
    }

    pub(in crate::agent_session) fn failure(&mut self, error: AgentSessionError) -> bool {
        self.publish_terminal(WorkerTerminal::Failure(error))
    }

    pub(in crate::agent_session) fn close(&mut self) -> bool {
        self.publish_terminal(WorkerTerminal::Closed)
    }

    fn publish_terminal(&self, terminal: WorkerTerminal) -> bool {
        {
            let mut published = self.terminal.lock().unwrap_or_else(PoisonError::into_inner);
            if published.is_none() {
                *published = Some(terminal);
            }
        }
        self.readiness.notify();
        true
    }
}
