use std::{collections::BTreeMap, path::Path};

use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentEvent, SessionId,
    TranscriptRecord,
    session_repository::{
        LocalSessionReader, SessionUsageReceipt, SessionUsageSource,
        UsageValue as StoredUsageValue, read_stored_session,
    },
};

use super::{
    PROVIDER_USAGE_SCHEMA, aggregate, binding,
    model::{
        ExternalRequest, ProviderUsageDocument, RawReceipt, SerializedTarget, UsageBinding,
        UsageFields, UsageReceipt, UsageSource, UsageTarget, UsageValue,
    },
};
use crate::review_protocol::digest;

pub(super) fn project(
    session_root: &Path,
    binding: UsageBinding,
) -> Result<ProviderUsageDocument, String> {
    let session_id = binding
        .session_id
        .parse::<SessionId>()
        .map_err(|error| format!("invalid usage-binding Session identity: {error}"))?;
    let reader = LocalSessionReader::open(session_root)
        .map_err(|error| format!("cannot open usage-binding Session repository: {error}"))?;
    let history = read_stored_session(&reader, session_id)
        .map_err(|error| format!("cannot recover usage-binding Session: {error}"))?;
    if history.descriptor().session_id() != session_id {
        return Err("usage receipt resolved to another Session".to_owned());
    }
    binding::require_request_binding(&history, &binding)?;

    let projection = history
        .session_usage()
        .map_err(|error| format!("cannot project exact delivery usage: {error}"))?;
    let raw = terminal_usage_snapshots(history.records());
    let mut receipts = Vec::new();
    for receipt in projection.receipts() {
        let activity = receipt.activity();
        if activity.session_id() != session_id {
            return Err("usage receipt changed the exact delivery Session".to_owned());
        }
        if activity.turn_id() != binding.turn_id {
            continue;
        }
        if !binding::source_matches(receipt.source(), &binding.target) {
            return Err("usage receipt changed the exact delivery target".to_owned());
        }
        let raw = raw.get(&activity).ok_or_else(|| {
            "exact delivery usage receipt has no terminal raw snapshot provenance".to_owned()
        })?;
        receipts.push(project_receipt(receipt, raw));
    }
    binding::require_source_request_binding(&receipts, &binding.target, &binding.request_id)?;

    let receipt_availability = if receipts.is_empty() {
        "unavailable"
    } else {
        "available"
    };
    let unavailable_reason = receipts
        .is_empty()
        .then_some("no_terminal_usage_receipt_for_exact_request_turn");
    let usage = aggregate::aggregate(&receipts)?;
    let analysis = aggregate::analyze(&usage, binding.packet_managed_tokens);
    let (request_kind, target) = match binding.target {
        UsageTarget::ManagedModel {
            provider,
            account,
            model,
        } => (
            "provider",
            SerializedTarget::ManagedModel {
                provider,
                account,
                model,
            },
        ),
        UsageTarget::DelegatedHost { host } => ("host", SerializedTarget::DelegatedHost { host }),
    };
    Ok(ProviderUsageDocument {
        schema: PROVIDER_USAGE_SCHEMA,
        review_id: binding.review_id,
        packet_hash: binding.packet_hash,
        request: ExternalRequest {
            kind: request_kind,
            id: binding.request_id,
        },
        target,
        session_id: binding.session_id,
        turn_id: binding.turn_id.get().get(),
        receipt_availability,
        unavailable_reason,
        receipts,
        usage,
        analysis,
    })
}

fn project_receipt(receipt: &SessionUsageReceipt, raw: &str) -> UsageReceipt {
    let source = match receipt.source() {
        SessionUsageSource::Managed {
            response_id,
            round,
            provider,
            account,
            model,
            connector,
            api_dialect,
            base_url,
        } => UsageSource::Managed {
            response_id: response_id.clone(),
            round: *round,
            provider: provider.clone(),
            account: account.clone(),
            model: model.clone(),
            connector: connector.clone(),
            api_dialect: api_dialect.clone(),
            base_url: base_url.clone(),
        },
        SessionUsageSource::Grok {
            source_profile,
            prompt_request_id,
        } => UsageSource::Grok {
            source_profile: source_profile.clone(),
            prompt_request_id: *prompt_request_id,
            model_calls: None,
            num_turns: None,
        },
        SessionUsageSource::GrokDiagnostic {
            source_profile,
            prompt_request_id,
            model_calls,
            num_turns,
        } => UsageSource::Grok {
            source_profile: source_profile.clone(),
            prompt_request_id: *prompt_request_id,
            model_calls: Some(*model_calls),
            num_turns: Some(*num_turns),
        },
        SessionUsageSource::Codex {
            source_profile,
            turn_id,
            model_context_window,
        } => UsageSource::Codex {
            source_profile: source_profile.clone(),
            turn_id: turn_id.clone(),
            model_context_window: *model_context_window,
        },
    };
    let usage = receipt.usage();
    UsageReceipt {
        receipt_schema: receipt.schema(),
        activity_id: receipt.activity().activity_id().get().get(),
        source,
        raw: RawReceipt {
            hash: digest(raw.as_bytes()),
            bytes: raw.len(),
        },
        usage: UsageFields {
            input_tokens: usage_value(usage.input_tokens()),
            output_tokens: usage_value(usage.output_tokens()),
            total_tokens: usage_value(usage.total_tokens()),
            reasoning_tokens: usage_value(usage.reasoning_tokens()),
            cache_read_input_tokens: usage_value(usage.cache_read_input_tokens()),
            cache_write_input_tokens: usage_value(usage.cache_write_input_tokens()),
        },
    }
}

fn terminal_usage_snapshots(records: &[TranscriptRecord]) -> BTreeMap<ActivityRef, String> {
    let mut pending = BTreeMap::<ActivityRef, (Option<String>, bool)>::new();
    let mut completed = BTreeMap::new();
    for record in records {
        let TranscriptRecord::EventCommitted(event) = record else {
            continue;
        };
        match event {
            AgentEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            } => {
                pending.insert(*activity, (None, false));
            },
            AgentEvent::ActivityUpdated { activity, update } => {
                let Some((text, final_snapshot)) = pending.get_mut(activity) else {
                    continue;
                };
                match update {
                    ActivityUpdate::TextSnapshot(snapshot) => {
                        *text = Some(snapshot.clone());
                        *final_snapshot = true;
                    },
                    ActivityUpdate::TextDelta(_) => *final_snapshot = false,
                }
            },
            AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            } => {
                if let Some((Some(text), true)) = pending.remove(activity) {
                    completed.insert(*activity, text);
                }
            },
            AgentEvent::ActivityFinished { activity, .. } => {
                pending.remove(activity);
            },
            AgentEvent::ActivityStarted { .. }
            | AgentEvent::SessionCreated { .. }
            | AgentEvent::TurnStarted { .. }
            | AgentEvent::TurnFinished { .. } => {},
        }
    }
    completed
}

const fn usage_value(value: StoredUsageValue) -> UsageValue {
    match value {
        StoredUsageValue::Reported(tokens) => UsageValue::Reported { tokens },
        StoredUsageValue::Absent => UsageValue::Absent,
        StoredUsageValue::Unsupported => UsageValue::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use yo_core::{
        ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentEvent,
        SessionUsageProjection, TranscriptRecord, TurnId, TurnRef,
    };

    use super::project_receipt;

    #[test]
    fn grok_diagnostics_survive_provider_usage_projection() {
        let session_id = "018f0a00-0000-7000-8000-000000000001".parse().unwrap();
        let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(1).unwrap()));
        let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
        let raw = serde_json::json!({
            "schema": "grok.acp-prompt-usage-receipt/v1alpha1",
            "source_profile": "grok.acp.prompt-response.meta-usage/v1",
            "prompt_request_id": 4,
            "model_calls": 7,
            "num_turns": 3,
            "usage": {
                "input_tokens": 100,
                "output_tokens": 10,
                "total_tokens": 110,
                "reasoning_tokens": 8,
                "cache_read_input_tokens": 60,
                "cache_write_input_tokens": 0
            }
        })
        .to_string();
        let records = vec![
            TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            }),
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(raw.clone()),
            }),
            TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            }),
        ];
        let projection = SessionUsageProjection::from_records(&records).unwrap();
        let projected =
            serde_json::to_value(project_receipt(&projection.receipts()[0], &raw)).unwrap();

        assert_eq!(
            projected["receipt_schema"],
            serde_json::json!("grok.acp-prompt-usage-receipt/v1alpha1")
        );
        assert_eq!(projected["source"]["model_calls"], serde_json::json!(7));
        assert_eq!(projected["source"]["num_turns"], serde_json::json!(3));
    }
}
