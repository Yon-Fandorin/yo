use serde::Serialize;
use yo_core::TurnId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::review_delivery) enum UsageTarget {
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
pub(in crate::review_delivery) struct UsageBinding {
    pub(in crate::review_delivery) review_id: String,
    pub(in crate::review_delivery) packet_hash: String,
    pub(in crate::review_delivery) packet_managed_tokens: usize,
    pub(in crate::review_delivery) request_id: String,
    pub(in crate::review_delivery) session_id: String,
    pub(in crate::review_delivery) turn_id: TurnId,
    pub(in crate::review_delivery) target: UsageTarget,
}

#[derive(Debug, Serialize)]
pub(in crate::review_delivery) struct ProviderUsageDocument {
    pub(super) schema: &'static str,
    pub(super) review_id: String,
    pub(super) packet_hash: String,
    pub(super) request: ExternalRequest,
    pub(super) target: SerializedTarget,
    pub(super) session_id: String,
    pub(super) turn_id: u64,
    pub(super) receipt_availability: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) unavailable_reason: Option<&'static str>,
    pub(super) receipts: Vec<UsageReceipt>,
    pub(super) usage: AggregatedUsage,
    pub(super) analysis: UsageAnalysis,
}

#[derive(Debug, Serialize)]
pub(super) struct ExternalRequest {
    pub(super) kind: &'static str,
    pub(super) id: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum SerializedTarget {
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
pub(super) struct UsageReceipt {
    pub(super) receipt_schema: &'static str,
    pub(super) activity_id: u64,
    pub(super) source: UsageSource,
    pub(super) raw: RawReceipt,
    pub(super) usage: UsageFields,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum UsageSource {
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
pub(super) struct RawReceipt {
    pub(super) hash: String,
    pub(super) bytes: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct UsageFields {
    pub(super) input_tokens: UsageValue,
    pub(super) output_tokens: UsageValue,
    pub(super) total_tokens: UsageValue,
    pub(super) reasoning_tokens: UsageValue,
    pub(super) cache_read_input_tokens: UsageValue,
    pub(super) cache_write_input_tokens: UsageValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
pub(super) enum UsageValue {
    Reported { tokens: u64 },
    Absent,
    Unsupported,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
pub(super) struct AggregatedUsage {
    pub(super) input_tokens: AggregateValue,
    pub(super) output_tokens: AggregateValue,
    pub(super) total_tokens: AggregateValue,
    pub(super) reasoning_tokens: AggregateValue,
    pub(super) cache_read_input_tokens: AggregateValue,
    pub(super) cache_write_input_tokens: AggregateValue,
}

#[derive(Debug, PartialEq, Serialize)]
pub(super) struct UsageAnalysis {
    pub(super) packet_managed_tokens: usize,
    pub(super) uncached_input_tokens: DerivedTokenValue,
    pub(super) input_amplification: InputAmplification,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
pub(super) enum DerivedTokenValue {
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
pub(super) enum InputAmplification {
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
pub(super) enum AggregateValue {
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
