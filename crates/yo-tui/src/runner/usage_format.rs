//! Shared terminal-independent text formatting for archived and live Usage views.

use yo_core::{
    CacheReadSummary, SessionUsageReceipt, SessionUsageSource, UsageAggregate, UsageCoverage,
    UsageValue,
};

pub(in crate::runner) fn aggregate_text(aggregate: UsageAggregate, receipt_count: usize) -> String {
    match aggregate.coverage() {
        UsageCoverage::Complete => aggregate
            .tokens()
            .map(format_tokens)
            .unwrap_or_else(|| unavailable_text(receipt_count)),
        UsageCoverage::Partial { reported, total } => aggregate
            .tokens()
            .map(|tokens| format!("{} (coverage={reported}/{total})", format_tokens(tokens)))
            .unwrap_or_else(|| format!("unavailable (coverage={reported}/{total})")),
        UsageCoverage::Unavailable => unavailable_text(receipt_count),
    }
}

pub(in crate::runner) fn cache_read_text(summary: CacheReadSummary) -> String {
    let value = match summary.share() {
        Some(share) => format!(
            "{}/{} ({}%)",
            format_tokens(share.cache_read_tokens()),
            format_tokens(share.input_tokens()),
            percentage(share.cache_read_tokens(), share.input_tokens()),
        ),
        None if summary.eligible_receipts() == 0 => format!(
            "unavailable ({}/{})",
            format_tokens(summary.cache_read_tokens()),
            format_tokens(summary.input_tokens()),
        ),
        None => format!(
            "{}/{} (percent unavailable)",
            format_tokens(summary.cache_read_tokens()),
            format_tokens(summary.input_tokens()),
        ),
    };
    format!(
        "{value} coverage={}/{}",
        summary.eligible_receipts(),
        summary.total_receipts()
    )
}

pub(in crate::runner) fn source_text(
    receipt: &SessionUsageReceipt,
    separator: Option<&str>,
) -> String {
    let divider = separator.map_or_else(|| " ".to_owned(), |value| format!(" {value} "));
    match receipt.source() {
        SessionUsageSource::Managed {
            provider,
            account,
            model,
            round,
            ..
        } => format!(
            "managed{divider}provider={} account={} model={} round={round}",
            safe_text(provider),
            safe_text(account),
            safe_text(model),
        ),
        SessionUsageSource::Grok {
            source_profile,
            prompt_request_id,
        }
        | SessionUsageSource::GrokDiagnostic {
            source_profile,
            prompt_request_id,
            ..
        } => format!(
            "grok{divider}profile={} request={prompt_request_id}",
            safe_text(source_profile),
        ),
        SessionUsageSource::Codex {
            source_profile,
            turn_id,
            model_context_window,
        } => {
            let context = model_context_window.map_or_else(String::new, |window| {
                format!(" context={}", format_tokens(window))
            });
            format!(
                "codex{divider}profile={} turn={}{}",
                safe_text(source_profile),
                safe_text(turn_id),
                context,
            )
        },
    }
}

pub(in crate::runner) fn value_text(value: UsageValue) -> String {
    match value {
        UsageValue::Reported(tokens) => format_tokens(tokens),
        UsageValue::Absent => "absent".to_owned(),
        UsageValue::Unsupported => "unsupported".to_owned(),
    }
}

pub(in crate::runner) fn safe_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            output.extend(character.escape_default());
        } else {
            output.push(character);
        }
    }
    output
}

fn unavailable_text(receipt_count: usize) -> String {
    format!("unavailable (coverage=0/{receipt_count})")
}

fn format_tokens(value: u64) -> String {
    let digits = value.to_string();
    let first_group_len = match digits.len() % 3 {
        0 => 3,
        remainder => remainder,
    };
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    output.push_str(&digits[..first_group_len]);
    for chunk in digits.as_bytes()[first_group_len..].chunks(3) {
        output.push(',');
        output.push_str(std::str::from_utf8(chunk).expect("token digits are valid UTF-8"));
    }
    output
}

fn percentage(numerator: u64, denominator: u64) -> u128 {
    u128::from(numerator) * 100 / u128::from(denominator)
}
