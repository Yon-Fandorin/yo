use serde_json::{Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{ActivityKind, ActivityOutcome, ActivityUpdate, BackendEvent, BackendFailure};

use super::super::{
    super::state::Backend,
    snapshots::{optional_non_negative_at, token_usage_breakdown_at, value_at},
};
use crate::protocol;

impl<P: JsonMessagePeer> Backend<P> {
    pub(super) fn token_usage_updated(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turnId"])?;
        let Some(turn) = self.wire_turns.get(wire_turn).map(|binding| binding.turn) else {
            // thread/resume은 과거 Codex Turn의 영속 사용량 스냅샷을 재생할 수 있습니다.
            // 해당 Turn에는 이 프로세스에서 신뢰할 수 있는 Yo Turn 바인딩이 없으므로,
            // 귀속을 임의로 만들지 말고 하위 경계를 정확히 유지합니다.
            return Ok(None);
        };
        let token_usage = value_at(params, &["tokenUsage"], "token usage")?;
        let last = token_usage_breakdown_at(token_usage, "last")?;
        let total = token_usage_breakdown_at(token_usage, "total")?;
        let model_context_window =
            optional_non_negative_at(token_usage, "modelContextWindow", "model context window")?;
        let receipt = json!({
            "schema": "codex.app-server-token-usage-receipt/v1",
            "source_profile": "codex.app-server.thread-token-usage-updated/v1",
            "turn_id": wire_turn,
            "usage": last.to_json(),
            "thread_total": total.to_json(),
            "model_context_window": model_context_window,
        });
        self.usage_activity(turn, receipt)
    }

    pub(super) fn usage_activity(
        &mut self,
        turn: yo_core::TurnRef,
        receipt: Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        let activity = self.next_activity(turn)?;
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(receipt.to_string()),
            });
        self.pending_events
            .push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            });
        Ok(Some(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ModelWork,
        }))
    }
}
