use std::{error::Error, fmt};

use super::coverage;
use crate::{ActivityRef, TranscriptRecord};

pub const MANAGED_USAGE_SCHEMA: &str = "yo.model-usage-receipt/v1";
pub const GROK_USAGE_SCHEMA: &str = "grok.acp-prompt-usage-receipt/v1";
pub const GROK_USAGE_DIAGNOSTIC_SCHEMA: &str = "grok.acp-prompt-usage-receipt/v1alpha1";
pub const CODEX_USAGE_SCHEMA: &str = "codex.app-server-token-usage-receipt/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionUsageProvider {
    Managed,
    Grok,
    Codex,
}

impl SessionUsageProvider {
    #[must_use]
    pub const fn schema(self) -> &'static str {
        match self {
            Self::Managed => MANAGED_USAGE_SCHEMA,
            Self::Grok => GROK_USAGE_SCHEMA,
            Self::Codex => CODEX_USAGE_SCHEMA,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionUsageSource {
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
    GrokDiagnostic {
        source_profile: String,
        prompt_request_id: u64,
        model_calls: u64,
        num_turns: u64,
    },
    Codex {
        source_profile: String,
        turn_id: String,
        model_context_window: Option<u64>,
    },
}

impl SessionUsageSource {
    #[must_use]
    pub const fn provider(&self) -> SessionUsageProvider {
        match self {
            Self::Managed { .. } => SessionUsageProvider::Managed,
            Self::Grok { .. } | Self::GrokDiagnostic { .. } => SessionUsageProvider::Grok,
            Self::Codex { .. } => SessionUsageProvider::Codex,
        }
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        match self {
            Self::GrokDiagnostic { .. } => GROK_USAGE_DIAGNOSTIC_SCHEMA,
            _ => self.provider().schema(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageValue {
    Reported(u64),
    Absent,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionUsage {
    pub(in super::super) input_tokens: UsageValue,
    pub(in super::super) output_tokens: UsageValue,
    pub(in super::super) total_tokens: UsageValue,
    pub(in super::super) reasoning_tokens: UsageValue,
    pub(in super::super) cache_read_input_tokens: UsageValue,
    pub(in super::super) cache_write_input_tokens: UsageValue,
}

impl SessionUsage {
    #[must_use]
    pub const fn input_tokens(&self) -> UsageValue {
        self.input_tokens
    }

    #[must_use]
    pub const fn output_tokens(&self) -> UsageValue {
        self.output_tokens
    }

    #[must_use]
    pub const fn total_tokens(&self) -> UsageValue {
        self.total_tokens
    }

    #[must_use]
    pub const fn reasoning_tokens(&self) -> UsageValue {
        self.reasoning_tokens
    }

    #[must_use]
    pub const fn cache_read_input_tokens(&self) -> UsageValue {
        self.cache_read_input_tokens
    }

    #[must_use]
    pub const fn cache_write_input_tokens(&self) -> UsageValue {
        self.cache_write_input_tokens
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageCoverage {
    Complete,
    Partial { reported: usize, total: usize },
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsageAggregate {
    pub(in super::super) tokens: u64,
    pub(in super::super) coverage: UsageCoverage,
}

impl UsageAggregate {
    #[must_use]
    pub const fn tokens(self) -> Option<u64> {
        match self.coverage {
            UsageCoverage::Unavailable => None,
            UsageCoverage::Complete | UsageCoverage::Partial { .. } => Some(self.tokens),
        }
    }

    #[must_use]
    pub const fn coverage(self) -> UsageCoverage {
        self.coverage
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionUsageAggregates {
    pub(in super::super) input_tokens: UsageAggregate,
    pub(in super::super) output_tokens: UsageAggregate,
    pub(in super::super) total_tokens: UsageAggregate,
    pub(in super::super) reasoning_tokens: UsageAggregate,
    pub(in super::super) cache_read_input_tokens: UsageAggregate,
}

impl SessionUsageAggregates {
    #[must_use]
    pub const fn input_tokens(self) -> UsageAggregate {
        self.input_tokens
    }

    #[must_use]
    pub const fn output_tokens(self) -> UsageAggregate {
        self.output_tokens
    }

    #[must_use]
    pub const fn total_tokens(self) -> UsageAggregate {
        self.total_tokens
    }

    #[must_use]
    pub const fn reasoning_tokens(self) -> UsageAggregate {
        self.reasoning_tokens
    }

    #[must_use]
    pub const fn cache_read_input_tokens(self) -> UsageAggregate {
        self.cache_read_input_tokens
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheReadShare {
    pub(super) cache_read_tokens: u64,
    pub(super) input_tokens: u64,
}

impl CacheReadShare {
    #[must_use]
    pub const fn cache_read_tokens(self) -> u64 {
        self.cache_read_tokens
    }

    #[must_use]
    pub const fn input_tokens(self) -> u64 {
        self.input_tokens
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheReadSummary {
    pub(in super::super) cache_read_tokens: u64,
    pub(in super::super) input_tokens: u64,
    pub(in super::super) eligible_receipts: usize,
    pub(in super::super) total_receipts: usize,
}

impl CacheReadSummary {
    #[must_use]
    pub const fn cache_read_tokens(self) -> u64 {
        self.cache_read_tokens
    }

    #[must_use]
    pub const fn input_tokens(self) -> u64 {
        self.input_tokens
    }

    #[must_use]
    pub const fn eligible_receipts(self) -> usize {
        self.eligible_receipts
    }

    #[must_use]
    pub const fn total_receipts(self) -> usize {
        self.total_receipts
    }

    #[must_use]
    pub const fn coverage(self) -> UsageCoverage {
        coverage(self.eligible_receipts, self.total_receipts)
    }

    #[must_use]
    pub const fn share(self) -> Option<CacheReadShare> {
        if self.input_tokens == 0 {
            None
        } else {
            Some(CacheReadShare {
                cache_read_tokens: self.cache_read_tokens,
                input_tokens: self.input_tokens,
            })
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionUsageReceipt {
    pub(in super::super) activity: ActivityRef,
    pub(in super::super) source: SessionUsageSource,
    pub(in super::super) usage: SessionUsage,
}

impl SessionUsageReceipt {
    #[must_use]
    pub const fn activity(&self) -> ActivityRef {
        self.activity
    }

    #[must_use]
    pub const fn source(&self) -> &SessionUsageSource {
        &self.source
    }

    #[must_use]
    pub const fn provider(&self) -> SessionUsageProvider {
        self.source.provider()
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        self.source.schema()
    }

    #[must_use]
    pub const fn usage(&self) -> &SessionUsage {
        &self.usage
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionUsageProjection {
    pub(in super::super) receipts: Vec<SessionUsageReceipt>,
    pub(in super::super) aggregates: SessionUsageAggregates,
    pub(in super::super) cache_read: CacheReadSummary,
}

impl SessionUsageProjection {
    #[must_use]
    pub fn receipts(&self) -> &[SessionUsageReceipt] {
        &self.receipts
    }

    #[must_use]
    pub const fn aggregates(&self) -> SessionUsageAggregates {
        self.aggregates
    }

    #[must_use]
    pub const fn cache_read(&self) -> CacheReadSummary {
        self.cache_read
    }

    #[must_use]
    pub const fn has_receipts(&self) -> bool {
        !self.receipts.is_empty()
    }

    pub fn from_records(records: &[TranscriptRecord]) -> Result<Self, SessionUsageError> {
        super::project_session_usage(records)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionUsageError {
    pub(in super::super) activity: ActivityRef,
    pub(in super::super) schema: String,
    pub(in super::super) detail: String,
}

impl SessionUsageError {
    #[must_use]
    pub const fn activity(&self) -> ActivityRef {
        self.activity
    }

    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    pub(in super::super) fn overflow(
        activity: ActivityRef,
        schema: &'static str,
        field: &'static str,
    ) -> Self {
        Self {
            activity,
            schema: schema.to_owned(),
            detail: format!("{field} aggregate overflowed u64"),
        }
    }
}

impl fmt::Display for SessionUsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid {} receipt for activity {:?}: {}",
            self.schema, self.activity, self.detail
        )
    }
}

impl Error for SessionUsageError {}
