use std::{collections::BTreeMap, path::Path};

use serde::Serialize;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentEvent, SessionId,
    TranscriptRecord, TurnId,
    session_repository::{
        LocalSessionReader, SessionUsageReceipt, SessionUsageSource, StoredRequestTraceRecord,
        UsageValue as StoredUsageValue, read_stored_session,
    },
};

use crate::{
    review_protocol::digest,
    review_session::{host_request_identity, provider_request_identity},
};

pub(super) const PROVIDER_USAGE_SCHEMA: &str = "yo.external-review-provider-usage/v1alpha1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum UsageTarget {
    ManagedModel {
        provider: String,
        account: String,
        model: String,
    },
    DelegatedHost {
        host: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct UsageBinding {
    pub(super) review_id: String,
    pub(super) packet_hash: String,
    pub(super) request_id: String,
    pub(super) session_id: String,
    pub(super) turn_id: TurnId,
    pub(super) target: UsageTarget,
}

#[derive(Debug, Serialize)]
pub(super) struct ProviderUsageDocument {
    schema: &'static str,
    review_id: String,
    packet_hash: String,
    request: ExternalRequest,
    target: SerializedTarget,
    session_id: String,
    turn_id: u64,
    receipt_availability: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    unavailable_reason: Option<&'static str>,
    receipts: Vec<UsageReceipt>,
    usage: AggregatedUsage,
}

#[derive(Debug, Serialize)]
struct ExternalRequest {
    kind: &'static str,
    id: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SerializedTarget {
    ManagedModel {
        provider: String,
        account: String,
        model: String,
    },
    DelegatedHost {
        host: String,
    },
}

#[derive(Clone, Debug, Serialize)]
struct UsageReceipt {
    receipt_schema: &'static str,
    activity_id: u64,
    source: UsageSource,
    raw: RawReceipt,
    usage: UsageFields,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum UsageSource {
    Managed {
        response_id: String,
        round: u64,
        provider: String,
        account: String,
        model: String,
        connector: String,
        api_dialect: String,
        base_url: String,
    },
    Grok {
        source_profile: String,
        prompt_request_id: u64,
    },
    Codex {
        source_profile: String,
        turn_id: String,
        model_context_window: Option<u64>,
    },
}

#[derive(Clone, Debug, Serialize)]
struct RawReceipt {
    hash: String,
    bytes: usize,
}

#[derive(Clone, Debug, Serialize)]
struct UsageFields {
    input_tokens: UsageValue,
    output_tokens: UsageValue,
    total_tokens: UsageValue,
    reasoning_tokens: UsageValue,
    cache_read_input_tokens: UsageValue,
    cache_write_input_tokens: UsageValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
enum UsageValue {
    Reported { tokens: u64 },
    Absent,
    Unsupported,
}

#[derive(Debug, Serialize)]
struct AggregatedUsage {
    input_tokens: AggregateValue,
    output_tokens: AggregateValue,
    total_tokens: AggregateValue,
    reasoning_tokens: AggregateValue,
    cache_read_input_tokens: AggregateValue,
    cache_write_input_tokens: AggregateValue,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
enum AggregateValue {
    Reported {
        tokens: u64,
    },
    Partial {
        tokens: u64,
        reported_receipts: usize,
        total_receipts: usize,
    },
    Unavailable {
        absent_receipts: usize,
        unsupported_receipts: usize,
        total_receipts: usize,
    },
}

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
    require_request_binding(&history, &binding)?;

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
        if !source_matches(receipt.source(), &binding.target) {
            return Err("usage receipt changed the exact delivery target".to_owned());
        }
        let raw = raw.get(&activity).ok_or_else(|| {
            "exact delivery usage receipt has no terminal raw snapshot provenance".to_owned()
        })?;
        receipts.push(project_receipt(receipt, raw));
    }
    require_source_request_binding(&receipts, &binding.target, &binding.request_id)?;

    let receipt_availability = if receipts.is_empty() {
        "unavailable"
    } else {
        "available"
    };
    let unavailable_reason = receipts
        .is_empty()
        .then_some("no_terminal_usage_receipt_for_exact_request_turn");
    let usage = aggregate(&receipts)?;
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
    })
}

fn require_request_binding(
    history: &yo_core::session_repository::StoredSessionHistory,
    binding: &UsageBinding,
) -> Result<(), String> {
    let mut requests = Vec::new();
    let mut outcomes = Vec::new();
    for entry in history.request_trace() {
        match entry.record() {
            StoredRequestTraceRecord::RequestAccepted {
                turn_id,
                request_identity,
                ..
            } if *turn_id == binding.turn_id => {
                requests.push(request_identity.value().to_owned());
            },
            StoredRequestTraceRecord::ResumableOutcome {
                turn_id,
                outcome_identity,
                ..
            } if *turn_id == binding.turn_id => outcomes.push(
                outcome_identity
                    .as_ref()
                    .map(|identity| identity.value().to_owned()),
            ),
            _ => {},
        }
    }
    require_bound_request_identity(&requests, &outcomes, &binding.target, &binding.request_id)
}

fn require_bound_request_identity(
    requests: &[String],
    outcomes: &[Option<String>],
    target: &UsageTarget,
    expected: &str,
) -> Result<(), String> {
    let observed = match target {
        UsageTarget::ManagedModel { .. } => provider_request_identity(requests, outcomes),
        UsageTarget::DelegatedHost { .. } => host_request_identity(requests, outcomes),
    }
    .map_err(|error| format!("usage binding has no exact request turn: {error}"))?;
    if observed == expected {
        Ok(())
    } else {
        Err(format!(
            "usage binding request identity mismatch: expected {}, found {observed}",
            expected
        ))
    }
}

fn source_matches(source: &SessionUsageSource, target: &UsageTarget) -> bool {
    match (source, target) {
        (
            SessionUsageSource::Managed {
                provider,
                account,
                model,
                ..
            },
            UsageTarget::ManagedModel {
                provider: expected_provider,
                account: expected_account,
                model: expected_model,
            },
        ) => {
            provider == expected_provider && account == expected_account && model == expected_model
        },
        (SessionUsageSource::Grok { .. }, UsageTarget::DelegatedHost { host }) => host == "grok",
        (SessionUsageSource::Codex { .. }, UsageTarget::DelegatedHost { host }) => host == "codex",
        _ => false,
    }
}

fn require_source_request_binding(
    receipts: &[UsageReceipt],
    target: &UsageTarget,
    request_id: &str,
) -> Result<(), String> {
    if receipts.is_empty() {
        return Ok(());
    }
    let matches = match target {
        UsageTarget::ManagedModel { .. } => matches!(
            receipts.last().map(|receipt| &receipt.source),
            Some(UsageSource::Managed { response_id, .. }) if response_id == request_id
        ),
        UsageTarget::DelegatedHost { host } if host == "grok" => {
            let identity: serde_json::Value = serde_json::from_str(request_id)
                .map_err(|error| format!("Grok usage request identity is not JSON: {error}"))?;
            let expected = identity
                .get("jsonRpcId")
                .and_then(serde_json::Value::as_u64);
            expected.is_some_and(|expected| {
                receipts.iter().all(|receipt| {
                    matches!(
                        receipt.source,
                        UsageSource::Grok {
                            prompt_request_id,
                            ..
                        } if prompt_request_id == expected
                    )
                })
            })
        },
        UsageTarget::DelegatedHost { host } if host == "codex" => {
            let identity: serde_json::Value = serde_json::from_str(request_id)
                .map_err(|error| format!("Codex usage request identity is not JSON: {error}"))?;
            let expected = identity.get("turnId").and_then(serde_json::Value::as_str);
            expected.is_some_and(|expected| {
                receipts.iter().all(|receipt| {
                    matches!(
                        &receipt.source,
                        UsageSource::Codex { turn_id, .. } if turn_id == expected
                    )
                })
            })
        },
        UsageTarget::DelegatedHost { .. } => false,
    };
    if matches {
        Ok(())
    } else {
        Err("usage source identity does not match the exact delivery request".to_owned())
    }
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

fn aggregate(receipts: &[UsageReceipt]) -> Result<AggregatedUsage, String> {
    Ok(AggregatedUsage {
        input_tokens: aggregate_field(receipts.iter().map(|receipt| receipt.usage.input_tokens))?,
        output_tokens: aggregate_field(receipts.iter().map(|receipt| receipt.usage.output_tokens))?,
        total_tokens: aggregate_field(receipts.iter().map(|receipt| receipt.usage.total_tokens))?,
        reasoning_tokens: aggregate_field(
            receipts
                .iter()
                .map(|receipt| receipt.usage.reasoning_tokens),
        )?,
        cache_read_input_tokens: aggregate_field(
            receipts
                .iter()
                .map(|receipt| receipt.usage.cache_read_input_tokens),
        )?,
        cache_write_input_tokens: aggregate_field(
            receipts
                .iter()
                .map(|receipt| receipt.usage.cache_write_input_tokens),
        )?,
    })
}

fn aggregate_field(values: impl IntoIterator<Item = UsageValue>) -> Result<AggregateValue, String> {
    let mut tokens = 0_u64;
    let mut reported = 0_usize;
    let mut absent = 0_usize;
    let mut unsupported = 0_usize;
    for value in values {
        match value {
            UsageValue::Reported { tokens: value } => {
                tokens = tokens
                    .checked_add(value)
                    .ok_or_else(|| "exact delivery usage aggregate overflowed u64".to_owned())?;
                reported += 1;
            },
            UsageValue::Absent => absent += 1,
            UsageValue::Unsupported => unsupported += 1,
        }
    }
    let total = reported + absent + unsupported;
    Ok(if reported == total && total > 0 {
        AggregateValue::Reported { tokens }
    } else if reported > 0 {
        AggregateValue::Partial {
            tokens,
            reported_receipts: reported,
            total_receipts: total,
        }
    } else {
        AggregateValue::Unavailable {
            absent_receipts: absent,
            unsupported_receipts: unsupported,
            total_receipts: total,
        }
    })
}

#[cfg(test)]
mod tests {
    use yo_core::SessionUsageSource;

    use super::{
        AggregateValue, UsageTarget, UsageValue, aggregate_field, require_bound_request_identity,
        source_matches,
    };

    // 모든 receipt가 값을 보고하면 합계는 완전한 reported 값이고, 하나라도 absent면
    // 알려진 합계를 버리지 않되 coverage를 partial로 명시합니다.
    #[test]
    fn aggregate_preserves_reported_and_partial_coverage() {
        assert_eq!(
            aggregate_field([
                UsageValue::Reported { tokens: 3 },
                UsageValue::Reported { tokens: 4 },
            ])
            .unwrap(),
            AggregateValue::Reported { tokens: 7 }
        );
        assert_eq!(
            aggregate_field([UsageValue::Reported { tokens: 3 }, UsageValue::Absent]).unwrap(),
            AggregateValue::Partial {
                tokens: 3,
                reported_receipts: 1,
                total_receipts: 2,
            }
        );
    }

    // receipt 자체가 없거나 필드가 전부 비보고 상태이면 0 token을 꾸며내지 않고
    // absent와 unsupported 개수를 포함한 unavailable 값으로 남깁니다.
    #[test]
    fn aggregate_keeps_unavailable_distinct_from_reported_zero() {
        assert_eq!(
            aggregate_field([]).unwrap(),
            AggregateValue::Unavailable {
                absent_receipts: 0,
                unsupported_receipts: 0,
                total_receipts: 0,
            }
        );
        assert_eq!(
            aggregate_field([UsageValue::Absent, UsageValue::Unsupported]).unwrap(),
            AggregateValue::Unavailable {
                absent_receipts: 1,
                unsupported_receipts: 1,
                total_receipts: 2,
            }
        );
        assert_eq!(
            aggregate_field([UsageValue::Reported { tokens: 0 }]).unwrap(),
            AggregateValue::Reported { tokens: 0 }
        );
    }

    // usage artifact는 같은 turn에서 관측한 exact outcome identity만 받으며, 다른 identity나
    // 중복 request를 가장 가까운 값으로 추측하지 않습니다.
    #[test]
    fn usage_binding_requires_one_exact_external_request_identity() {
        let target = UsageTarget::ManagedModel {
            provider: "kimi".to_owned(),
            account: "default".to_owned(),
            model: "k3".to_owned(),
        };
        let requests = ["request-1".to_owned()];
        let outcomes = [Some("response-1".to_owned())];
        assert!(
            require_bound_request_identity(&requests, &outcomes, &target, "response-1").is_ok()
        );
        assert!(
            require_bound_request_identity(&requests, &outcomes, &target, "request-1").is_err()
        );
        assert!(
            require_bound_request_identity(
                &["request-1".to_owned(), "request-2".to_owned()],
                &outcomes,
                &target,
                "response-1",
            )
            .is_err()
        );
    }

    // Kimi/Qwen managed route와 Grok/Codex delegated route는 각각 자기 source variant와
    // exact 좌표에만 대응하며 다른 Provider나 host의 영수증을 재사용하지 않습니다.
    #[test]
    fn usage_source_matching_covers_every_external_review_route() {
        let managed = SessionUsageSource::Managed {
            response_id: "response-1".to_owned(),
            round: 1,
            provider: "kimi".to_owned(),
            account: "default".to_owned(),
            model: "k3".to_owned(),
            connector: "kimi".to_owned(),
            api_dialect: "chat-completions".to_owned(),
            base_url: "https://example.invalid".to_owned(),
        };
        assert!(source_matches(
            &managed,
            &UsageTarget::ManagedModel {
                provider: "kimi".to_owned(),
                account: "default".to_owned(),
                model: "k3".to_owned(),
            }
        ));
        assert!(!source_matches(
            &managed,
            &UsageTarget::ManagedModel {
                provider: "qwencloud".to_owned(),
                account: "default".to_owned(),
                model: "qwen3.8-max".to_owned(),
            }
        ));

        let grok = SessionUsageSource::Grok {
            source_profile: "grok.acp.prompt-response.usage/v1".to_owned(),
            prompt_request_id: 7,
        };
        let codex = SessionUsageSource::Codex {
            source_profile: "codex.app-server.thread-token-usage-updated/v1".to_owned(),
            turn_id: "turn-1".to_owned(),
            model_context_window: Some(258_000),
        };
        assert!(source_matches(
            &grok,
            &UsageTarget::DelegatedHost {
                host: "grok".to_owned(),
            }
        ));
        assert!(source_matches(
            &codex,
            &UsageTarget::DelegatedHost {
                host: "codex".to_owned(),
            }
        ));
        assert!(!source_matches(
            &codex,
            &UsageTarget::DelegatedHost {
                host: "grok".to_owned(),
            }
        ));
    }
}
