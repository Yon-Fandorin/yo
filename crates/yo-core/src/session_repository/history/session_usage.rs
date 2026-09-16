//! Receipt models, parsers, and semantic projections for stored Session history.

mod model;
mod projection;
mod receipts;

#[cfg(test)]
mod tests;

pub(super) use model::GROK_USAGE_DIAGNOSTIC_SCHEMA;
pub use model::{
    CODEX_USAGE_SCHEMA, CacheReadShare, CacheReadSummary, GROK_USAGE_SCHEMA, MANAGED_USAGE_SCHEMA,
    SessionUsage, SessionUsageAggregates, SessionUsageError, SessionUsageProjection,
    SessionUsageProvider, SessionUsageReceipt, SessionUsageSource, UsageAggregate, UsageCoverage,
    UsageValue,
};
pub(super) use projection::{build_projection, coverage, project_session_usage};
pub(super) use receipts::{
    closed_object, closed_object_at, object_at, optional_root_u64, optional_usage, parse_codex,
    parse_grok, parse_grok_diagnostic, parse_grok_fields, parse_managed, parse_managed_cache,
    parse_receipt, required_profile_id, required_root_u64, required_string, required_u64,
    required_usage, validate_closed_fields,
};
