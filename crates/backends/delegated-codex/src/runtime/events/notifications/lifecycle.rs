use yo_backend::transport::JsonMessagePeer;
use yo_core::{ActivityOutcome, BackendEvent, BackendFailure, BackendPoll};

use super::super::super::state::Backend;

impl<P: JsonMessagePeer> Backend<P> {
    pub(super) fn close_connection(
        &mut self,
        terminal: Result<(), BackendFailure>,
    ) -> Result<BackendPoll, BackendFailure> {
        // 런타임의 최종 실패가 턴을 닫기 전에 인터뷰 영수증을 전달합니다.
        // 전송이 끊긴 뒤 기록된 로컬 답변을 제출된 것으로 보고하지 않습니다.
        let mut requests = self.requests.keys().copied().collect::<Vec<_>>();
        requests.sort_unstable_by_key(|request| request.activity());
        let mut events = Vec::new();
        for request in requests {
            let summaries = self.interview_summary_events(
                request,
                "Connection to Codex ended before submission completed.",
            )?;
            let binding = &self.requests[&request];
            events.push(BackendEvent::ActivityFinished {
                activity: binding.request_activity,
                outcome: if binding.responded {
                    ActivityOutcome::Completed
                } else {
                    ActivityOutcome::Interrupted
                },
            });
            events.extend(summaries);
        }
        self.requests.clear();
        self.wire_requests.clear();
        self.pending_events.extend(events);
        self.terminal_poll = Some(terminal.clone());
        match self.pending_events.pop_front() {
            Some(event) => Ok(BackendPoll::Event(event)),
            None => terminal.map(|()| BackendPoll::Closed),
        }
    }
}
