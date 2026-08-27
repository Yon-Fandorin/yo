use serde::{Deserialize, Serialize};

pub(super) const REQUEST_SCHEMA: &str = "yo.external-review-target-admission-request/v1alpha1";
pub(super) const RESULT_SCHEMA: &str = "yo.external-review-target-admission-result/v1alpha1";
pub(super) const REQUEST_SCHEMA_V1_ALPHA2: &str =
    "yo.external-review-target-admission-request/v1alpha2";
pub(super) const RESULT_SCHEMA_V1_ALPHA2: &str =
    "yo.external-review-target-admission-result/v1alpha2";
pub(super) const REQUEST_SCHEMA_V1_ALPHA3: &str =
    "yo.external-review-target-admission-request/v1alpha3";
pub(super) const RESULT_SCHEMA_V1_ALPHA3: &str =
    "yo.external-review-target-admission-result/v1alpha3";

pub(super) fn result_schema(request_schema: &str) -> &'static str {
    match request_schema {
        REQUEST_SCHEMA => RESULT_SCHEMA,
        REQUEST_SCHEMA_V1_ALPHA2 => RESULT_SCHEMA_V1_ALPHA2,
        REQUEST_SCHEMA_V1_ALPHA3 => RESULT_SCHEMA_V1_ALPHA3,
        _ => unreachable!("validated admission schema"),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ReviewTarget {
    ManagedModel {
        provider: String,
        account: String,
        model: String,
    },
    DelegatedHost {
        host: String,
    },
}

impl ReviewTarget {
    pub(crate) fn managed(provider: String, account: String, model: String) -> Self {
        Self::ManagedModel {
            provider,
            account,
            model,
        }
    }

    pub(crate) fn reference(&self) -> String {
        match self {
            Self::ManagedModel {
                provider,
                account,
                model,
            } => format!("{provider}:{account}:{model}"),
            Self::DelegatedHost { host } => format!("host:{host}"),
        }
    }

    pub(super) fn admitted_outcome(&self, request_schema: &str) -> (&'static str, &'static str) {
        match (request_schema, self) {
            (_, Self::ManagedModel { .. }) => ("eligible", "deliver_once"),
            (REQUEST_SCHEMA, Self::DelegatedHost { .. }) => {
                ("prepared", "await_delegated_delivery_protocol")
            },
            (REQUEST_SCHEMA_V1_ALPHA2, Self::DelegatedHost { .. }) => {
                ("eligible", "deliver_delegated_once")
            },
            (REQUEST_SCHEMA_V1_ALPHA3, Self::DelegatedHost { .. }) => {
                ("eligible", "deliver_delegated_once")
            },
            _ => unreachable!("validated admission schema"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) schema: String,
    pub(super) target: ReviewTarget,
    #[serde(default)]
    pub(super) connection_repository_path: Option<String>,
    #[serde(default)]
    pub(super) session_repository_path: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Decision {
    Admit,
    Stop,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct Availability {
    pub(super) state: &'static str,
    pub(super) source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) failure_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) observed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) executable: Option<String>,
    pub(super) detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct AccountLimit {
    pub(super) availability: &'static str,
    pub(super) remaining: Option<u64>,
    pub(super) resets_at: Option<String>,
    pub(super) source: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct UsageSearch {
    pub(super) state: &'static str,
    pub(super) inspected_sessions: usize,
    pub(super) truncated: bool,
    pub(super) selection_basis: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) detail: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct UsageReceipt {
    pub(super) session_id: String,
    pub(super) session_updated_unix_millis: u64,
    pub(super) schema: &'static str,
    pub(super) source: serde_json::Value,
    pub(super) usage: UsageFields,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct UsageFields {
    pub(super) input_tokens: UsageValue,
    pub(super) output_tokens: UsageValue,
    pub(super) total_tokens: UsageValue,
    pub(super) reasoning_tokens: UsageValue,
    pub(super) cache_read_input_tokens: UsageValue,
    pub(super) cache_write_input_tokens: UsageValue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
pub(super) enum UsageValue {
    Reported { tokens: u64 },
    Absent,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct Admission {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) status: &'static str,
    pub(super) next_action: &'static str,
    pub(super) decision: Decision,
    pub(crate) target: ReviewTarget,
    pub(super) target_reference: String,
    pub(super) availability: Availability,
    pub(super) account_limit: AccountLimit,
    pub(super) usage_search: UsageSearch,
    pub(super) last_exact_usage_receipt: Option<UsageReceipt>,
}

impl Admission {
    pub(crate) const fn admitted(&self) -> bool {
        matches!(self.decision, Decision::Admit)
    }

    pub(crate) fn availability_detail(&self) -> &str {
        &self.availability.detail
    }

    pub(crate) fn supports_frozen_delegated_delivery(&self) -> bool {
        self.schema == RESULT_SCHEMA_V1_ALPHA2
            && matches!(self.target, ReviewTarget::DelegatedHost { .. })
            && matches!(self.decision, Decision::Admit)
    }

    pub(crate) fn has_delegated_host_state_readiness(&self) -> bool {
        self.schema == RESULT_SCHEMA_V1_ALPHA3
            && matches!(self.target, ReviewTarget::DelegatedHost { .. })
            && matches!(self.decision, Decision::Admit)
    }
}
