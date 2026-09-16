use std::collections::BTreeMap;

use super::{
    CacheReadSummary, SessionUsage, SessionUsageAggregates, SessionUsageError,
    SessionUsageProjection, SessionUsageReceipt, UsageAggregate, UsageCoverage, UsageValue,
    parse_receipt,
};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentEvent, TranscriptRecord,
};

#[derive(Default)]
struct PendingModelWork {
    text: Option<String>,
    final_update_was_snapshot: bool,
}

pub(super) fn project_session_usage(
    records: &[TranscriptRecord],
) -> Result<SessionUsageProjection, SessionUsageError> {
    let mut pending = BTreeMap::<ActivityRef, PendingModelWork>::new();
    let mut receipts = Vec::new();

    for record in records {
        let TranscriptRecord::EventCommitted(event) = record else {
            continue;
        };
        match event {
            AgentEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            } => {
                pending.insert(*activity, PendingModelWork::default());
            },
            AgentEvent::ActivityStarted { .. } => {},
            AgentEvent::ActivityUpdated { activity, update } => {
                let Some(work) = pending.get_mut(activity) else {
                    continue;
                };
                match update {
                    ActivityUpdate::TextSnapshot(text) => {
                        work.text = Some(text.clone());
                        work.final_update_was_snapshot = true;
                    },
                    ActivityUpdate::TextDelta(_) => {
                        work.final_update_was_snapshot = false;
                    },
                }
            },
            AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            } => {
                let Some(work) = pending.remove(activity) else {
                    continue;
                };
                if !work.final_update_was_snapshot {
                    continue;
                }
                let Some(text) = work.text else {
                    continue;
                };
                if let Some(receipt) = parse_receipt(&text, *activity)? {
                    receipts.push(receipt);
                }
            },
            AgentEvent::ActivityFinished { activity, .. } => {
                pending.remove(activity);
            },
            AgentEvent::SessionCreated { .. }
            | AgentEvent::TurnStarted { .. }
            | AgentEvent::TurnFinished { .. } => {},
        }
    }

    build_projection(receipts)
}

pub(in super::super) fn build_projection(
    receipts: Vec<SessionUsageReceipt>,
) -> Result<SessionUsageProjection, SessionUsageError> {
    let total_receipts = receipts.len();
    let aggregates = SessionUsageAggregates {
        input_tokens: aggregate(&receipts, "input_tokens", |usage| usage.input_tokens)?,
        output_tokens: aggregate(&receipts, "output_tokens", |usage| usage.output_tokens)?,
        total_tokens: aggregate(&receipts, "total_tokens", |usage| usage.total_tokens)?,
        reasoning_tokens: aggregate(&receipts, "reasoning_tokens", |usage| {
            usage.reasoning_tokens
        })?,
        cache_read_input_tokens: aggregate(&receipts, "cache_read_input_tokens", |usage| {
            usage.cache_read_input_tokens
        })?,
    };

    let mut cache_read_tokens = 0_u64;
    let mut input_tokens = 0_u64;
    let mut eligible_receipts = 0_usize;
    for receipt in &receipts {
        let (UsageValue::Reported(cache_read), UsageValue::Reported(input)) = (
            receipt.usage.cache_read_input_tokens,
            receipt.usage.input_tokens,
        ) else {
            continue;
        };
        cache_read_tokens = cache_read_tokens.checked_add(cache_read).ok_or_else(|| {
            SessionUsageError::overflow(
                receipt.activity,
                receipt.schema(),
                "cache_read_input_tokens",
            )
        })?;
        input_tokens = input_tokens.checked_add(input).ok_or_else(|| {
            SessionUsageError::overflow(receipt.activity, receipt.schema(), "input_tokens")
        })?;
        eligible_receipts += 1;
    }

    Ok(SessionUsageProjection {
        receipts,
        aggregates,
        cache_read: CacheReadSummary {
            cache_read_tokens,
            input_tokens,
            eligible_receipts,
            total_receipts,
        },
    })
}

fn aggregate(
    receipts: &[SessionUsageReceipt],
    field: &'static str,
    value: impl Fn(&SessionUsage) -> UsageValue,
) -> Result<UsageAggregate, SessionUsageError> {
    let mut total = 0_u64;
    let mut reported = 0_usize;
    let mut count = 0_usize;
    for receipt in receipts {
        count += 1;
        if let UsageValue::Reported(tokens) = value(&receipt.usage) {
            total = total.checked_add(tokens).ok_or_else(|| {
                SessionUsageError::overflow(receipt.activity, receipt.schema(), field)
            })?;
            reported += 1;
        }
    }
    Ok(UsageAggregate {
        tokens: total,
        coverage: coverage(reported, count),
    })
}

pub(super) const fn coverage(reported: usize, total: usize) -> UsageCoverage {
    match (reported, total) {
        (0, _) => UsageCoverage::Unavailable,
        (reported, total) if reported == total => UsageCoverage::Complete,
        (reported, total) => UsageCoverage::Partial { reported, total },
    }
}
