//! 저장된 세션 이력의 영수증 모델, 파서, 의미 투영.

mod model;
mod projection;
mod receipts;

#[cfg(test)]
mod tests;

use model::GROK_USAGE_DIAGNOSTIC_SCHEMA;
pub use model::{
    CODEX_USAGE_SCHEMA, CacheReadShare, CacheReadSummary, GROK_USAGE_SCHEMA, MANAGED_USAGE_SCHEMA,
    SessionUsage, SessionUsageAggregates, SessionUsageError, SessionUsageProjection,
    SessionUsageProvider, SessionUsageReceipt, SessionUsageSource, UsageAggregate, UsageCoverage,
    UsageValue,
};
use projection::{coverage, project_session_usage};
