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
    review::session::{host_request_identity, provider_request_identity},
    review_protocol::digest,
};

pub(super) const PROVIDER_USAGE_SCHEMA: &str = "yo.external-review-provider-usage/v1alpha2";

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
    pub(super) packet_managed_tokens: usize,
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
    analysis: UsageAnalysis,
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
        #[serde(skip_serializing_if = "Option::is_none")]
        model_calls: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        num_turns: Option<u64>,
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

#[derive(Debug, Eq, PartialEq, Serialize)]
struct AggregatedUsage {
    input_tokens: AggregateValue,
    output_tokens: AggregateValue,
    total_tokens: AggregateValue,
    reasoning_tokens: AggregateValue,
    cache_read_input_tokens: AggregateValue,
    cache_write_input_tokens: AggregateValue,
}

#[derive(Debug, PartialEq, Serialize)]
struct UsageAnalysis {
    packet_managed_tokens: usize,
    uncached_input_tokens: DerivedTokenValue,
    input_amplification: InputAmplification,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
enum DerivedTokenValue {
    Reported {
        tokens: u64,
        derivation: &'static str,
    },
    Unavailable {
        reason: &'static str,
    },
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
enum InputAmplification {
    Reported {
        ratio: f64,
        provider_input_tokens: u64,
        packet_managed_tokens: usize,
    },
    Unavailable {
        reason: &'static str,
    },
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
    let analysis = analyze(&usage, binding.packet_managed_tokens);
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
        (
            SessionUsageSource::Grok { .. } | SessionUsageSource::GrokDiagnostic { .. },
            UsageTarget::DelegatedHost { host },
        ) => host == "grok",
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

fn analyze(usage: &AggregatedUsage, packet_managed_tokens: usize) -> UsageAnalysis {
    let uncached_input_tokens = match (&usage.input_tokens, &usage.cache_read_input_tokens) {
        (
            AggregateValue::Reported { tokens: input },
            AggregateValue::Reported { tokens: cache_read },
        ) if cache_read <= input => DerivedTokenValue::Reported {
            tokens: input - cache_read,
            derivation: "input_tokens-cache_read_input_tokens",
        },
        (
            AggregateValue::Reported { tokens: input },
            AggregateValue::Reported { tokens: cache_read },
        ) if cache_read > input => DerivedTokenValue::Unavailable {
            reason: "cache_read_exceeds_input",
        },
        _ => DerivedTokenValue::Unavailable {
            reason: "incomplete_input_or_cache_read_coverage",
        },
    };
    let input_amplification = match (&usage.input_tokens, packet_managed_tokens) {
        (_, 0) => InputAmplification::Unavailable {
            reason: "packet_managed_tokens_zero",
        },
        (AggregateValue::Reported { tokens }, packet_managed_tokens) => {
            let ratio =
                ((*tokens as f64 / packet_managed_tokens as f64) * 1_000.0).round() / 1_000.0;
            InputAmplification::Reported {
                ratio,
                provider_input_tokens: *tokens,
                packet_managed_tokens,
            }
        },
        _ => InputAmplification::Unavailable {
            reason: "incomplete_input_coverage",
        },
    };
    UsageAnalysis {
        packet_managed_tokens,
        uncached_input_tokens,
        input_amplification,
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use yo_core::{
        ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentEvent,
        SessionUsageProjection, SessionUsageSource, TranscriptRecord, TurnId, TurnRef,
    };

    use super::{
        AggregateValue, AggregatedUsage, DerivedTokenValue, InputAmplification, UsageAnalysis,
        UsageTarget, UsageValue, aggregate_field, analyze, project_receipt,
        require_bound_request_identity, source_matches,
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

    // packet 크기와 완전하게 보고된 provider/cache 입력에서만 비캐시 입력과 읽기 쉬운
    // 3자리 증폭률을 파생하고, cache token을 Provider 입력에 다시 더하지 않습니다.
    #[test]
    fn analysis_separates_uncached_input_and_packet_amplification() {
        let usage = complete_usage(990_347, 585_728);
        assert_eq!(
            analyze(&usage, 43_276),
            UsageAnalysis {
                packet_managed_tokens: 43_276,
                uncached_input_tokens: DerivedTokenValue::Reported {
                    tokens: 404_619,
                    derivation: "input_tokens-cache_read_input_tokens",
                },
                input_amplification: InputAmplification::Reported {
                    ratio: 22.884,
                    provider_input_tokens: 990_347,
                    packet_managed_tokens: 43_276,
                },
            }
        );
    }

    // Grok diagnostic receipt의 새 schema와 host-work 계수는 shared Session projection을
    // 통과한 뒤에도 external-review Provider Usage source에 손실 없이 직렬화됩니다.
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

    // 불완전한 coverage, 0-byte packet, 또는 input보다 큰 cache 보고는 0이나 음수를
    // 꾸며내지 않고 각 파생치의 구체적인 unavailable 이유로 남깁니다.
    #[test]
    fn analysis_fails_closed_when_inputs_do_not_support_a_derivation() {
        let mut partial = complete_usage(10, 4);
        partial.input_tokens = AggregateValue::Partial {
            tokens: 10,
            reported_receipts: 1,
            total_receipts: 2,
        };
        assert_eq!(
            analyze(&partial, 5),
            UsageAnalysis {
                packet_managed_tokens: 5,
                uncached_input_tokens: DerivedTokenValue::Unavailable {
                    reason: "incomplete_input_or_cache_read_coverage",
                },
                input_amplification: InputAmplification::Unavailable {
                    reason: "incomplete_input_coverage",
                },
            }
        );

        let invalid_cache = complete_usage(3, 4);
        assert_eq!(
            analyze(&invalid_cache, 0),
            UsageAnalysis {
                packet_managed_tokens: 0,
                uncached_input_tokens: DerivedTokenValue::Unavailable {
                    reason: "cache_read_exceeds_input",
                },
                input_amplification: InputAmplification::Unavailable {
                    reason: "packet_managed_tokens_zero",
                },
            }
        );
    }

    fn complete_usage(input_tokens: u64, cache_read_input_tokens: u64) -> AggregatedUsage {
        AggregatedUsage {
            input_tokens: AggregateValue::Reported {
                tokens: input_tokens,
            },
            output_tokens: AggregateValue::Reported { tokens: 0 },
            total_tokens: AggregateValue::Reported {
                tokens: input_tokens,
            },
            reasoning_tokens: AggregateValue::Reported { tokens: 0 },
            cache_read_input_tokens: AggregateValue::Reported {
                tokens: cache_read_input_tokens,
            },
            cache_write_input_tokens: AggregateValue::Reported { tokens: 0 },
        }
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
