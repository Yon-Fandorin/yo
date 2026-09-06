use super::{
    ChatCompletionsSseDecoder, ConnectorError, KIMI_PRIVATE_MESSAGE_FIXED_BYTES,
    KIMI_TOOL_CALLS_FIELD_BYTES, KimiReplayToolCallSize, kimi_replay_round_item_lengths,
    limit_failure, wire::bounded_sum,
};

impl ChatCompletionsSseDecoder {
    pub(super) fn add_content_bytes(&mut self, bytes: usize) -> Result<(), ConnectorError> {
        self.content_bytes = bounded_sum(
            self.content_bytes,
            bytes,
            self.limits.max_response_text_bytes,
            "Chat Completions content byte limit exceeded",
        )?;
        Ok(())
    }

    pub(super) fn add_refusal_bytes(&mut self, bytes: usize) -> Result<(), ConnectorError> {
        self.refusal_bytes = bounded_sum(
            self.refusal_bytes,
            bytes,
            self.limits.max_refusal_bytes,
            "Chat Completions refusal byte limit exceeded",
        )?;
        Ok(())
    }

    pub(super) fn add_reasoning_bytes(&mut self, bytes: usize) -> Result<(), ConnectorError> {
        self.reasoning_bytes = bounded_sum(
            self.reasoning_bytes,
            bytes,
            self.limits.max_reasoning_bytes,
            "Chat Completions reasoning byte limit exceeded",
        )?;
        Ok(())
    }

    pub(super) fn add_argument_bytes(&mut self, bytes: usize) -> Result<(), ConnectorError> {
        self.argument_bytes = bounded_sum(
            self.argument_bytes,
            bytes,
            self.limits.max_function_argument_bytes,
            "Chat Completions function-argument byte limit exceeded",
        )?;
        Ok(())
    }

    pub(super) fn ensure_kimi_private_budget(
        &self,
        content_seen: bool,
        content_encoded_bytes: usize,
        reasoning_encoded_bytes: usize,
        tool_calls_encoded_bytes: usize,
    ) -> Result<(), ConnectorError> {
        let content_bytes = if content_seen {
            content_encoded_bytes.checked_add(2)
        } else {
            Some(4)
        };
        let mut total = KIMI_PRIVATE_MESSAGE_FIXED_BYTES
            .checked_add(
                content_bytes
                    .ok_or_else(|| limit_failure("Kimi private assistant byte limit exceeded"))?,
            )
            .and_then(|total| total.checked_add(reasoning_encoded_bytes));
        if tool_calls_encoded_bytes != 0 {
            total = total
                .and_then(|total| total.checked_add(KIMI_TOOL_CALLS_FIELD_BYTES))
                .and_then(|total| total.checked_add(tool_calls_encoded_bytes));
        }
        if total.is_none_or(|total| total > self.limits.max_provider_private_bytes) {
            return Err(limit_failure("Kimi private assistant byte limit exceeded"));
        }
        Ok(())
    }

    pub(super) fn ensure_kimi_complete_replay_budget(
        &self,
        content_seen: bool,
        content_encoded_bytes: usize,
        reasoning_encoded_bytes: usize,
        updated_call: Option<(usize, KimiReplayToolCallSize)>,
    ) -> Result<(), ConnectorError> {
        let replay_budget = self.replay_budget;
        let call_indices = self
            .calls
            .keys()
            .copied()
            .chain(updated_call.map(|(index, _)| index))
            .collect::<std::collections::BTreeSet<_>>();
        let call_sizes = call_indices
            .into_iter()
            .map(|index| {
                updated_call
                    .filter(|(updated_index, _)| *updated_index == index)
                    .map(|(_, size)| size)
                    .or_else(|| self.calls.get(&index).map(|call| call.encoded_size))
                    .expect("every retained Kimi call has an encoded size")
            })
            .collect::<Vec<_>>();
        let item_lengths = kimi_replay_round_item_lengths(
            self.kimi_private_replay(),
            content_seen,
            content_encoded_bytes,
            reasoning_encoded_bytes,
            &call_sizes,
        )
        .ok_or_else(|| limit_failure("Kimi complete replay byte limit exceeded"))?;
        if replay_budget.accepts_item_lengths(&item_lengths) {
            Ok(())
        } else {
            Err(limit_failure("Kimi complete replay byte limit exceeded"))
        }
    }
}
