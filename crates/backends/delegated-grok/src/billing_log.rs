use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use jiff::Timestamp;
use serde_json::Value;
use yo_core::{AccountCapacityWindow, BackendFailure, BackendFailureKind};

const MAX_TAIL_BYTES: u64 = 1024 * 1024;
const MAX_LINE_BYTES: usize = 64 * 1024;
const WEEKLY_MINUTES: u64 = 7 * 24 * 60;
const BILLING_MESSAGE: &str = "billing: fetched credits config";

/// Reads only a bounded tail and accepts the newest complete official billing-log event.
///
/// Contract source: xAI Grok Build commit 9684fa3, `extensions/billing.rs`, whose
/// successful billing fetch writes this exact structured event to `unified.jsonl`.
pub(super) fn read_latest_usage(
    path: &Path,
) -> Result<Option<AccountCapacityWindow>, BackendFailure> {
    let mut file = File::open(path).map_err(io_failure)?;
    let length = file.metadata().map_err(io_failure)?.len();
    let start = length.saturating_sub(MAX_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).map_err(io_failure)?;
    let mut tail = Vec::with_capacity((length - start).min(MAX_TAIL_BYTES) as usize);
    file.take(MAX_TAIL_BYTES)
        .read_to_end(&mut tail)
        .map_err(io_failure)?;

    let complete = if start == 0 {
        tail.as_slice()
    } else {
        tail.iter()
            .position(|byte| *byte == b'\n')
            .map_or(&[][..], |index| &tail[index + 1..])
    };
    for line in complete.split(|byte| *byte == b'\n').rev() {
        if line.is_empty() || line.len() > MAX_LINE_BYTES {
            continue;
        }
        let Ok(event) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if event.get("msg").and_then(Value::as_str) != Some(BILLING_MESSAGE) {
            continue;
        }
        if let Some(window) = decode_usage(&event)? {
            return Ok(Some(window));
        }
    }
    Ok(None)
}

fn decode_usage(event: &Value) -> Result<Option<AccountCapacityWindow>, BackendFailure> {
    let Some(config) = event.pointer("/ctx/config").and_then(Value::as_object) else {
        return Ok(None);
    };
    let Some(period) = config.get("currentPeriod").and_then(Value::as_object) else {
        return Ok(None);
    };
    if period.get("type").and_then(Value::as_str) != Some("USAGE_PERIOD_TYPE_WEEKLY") {
        return Ok(None);
    }
    let Some(end) = period.get("end").and_then(Value::as_str) else {
        return Ok(None);
    };
    let reset = end
        .parse::<Timestamp>()
        .map_err(|_| protocol_failure("Grok billing log currentPeriod.end is not RFC 3339"))?;
    if reset <= Timestamp::now() {
        return Ok(None);
    }

    // Grok's proto JSON omits zero-valued scalars. A valid current period with an
    // absent percentage therefore means zero used, matching Grok's own TUI.
    let used_percent_basis_points = match config.get("creditUsagePercent") {
        None => 0,
        Some(value) => {
            let value = value.as_f64().ok_or_else(|| {
                protocol_failure("Grok billing log creditUsagePercent is not numeric")
            })?;
            if !value.is_finite() || !(0.0..=100.0).contains(&value) {
                return Err(protocol_failure(
                    "Grok billing log creditUsagePercent is outside 0..=100",
                ));
            }
            (value * 100.0).ceil().min(10_000.0) as u16
        },
    };
    AccountCapacityWindow::from_used_percent_basis_points(
        used_percent_basis_points,
        Some(WEEKLY_MINUTES),
        Some(reset.as_second()),
    )
    .map(Some)
    .map_err(|error| protocol_failure(error.to_string()))
}

fn io_failure(error: std::io::Error) -> BackendFailure {
    BackendFailure::new(
        BackendFailureKind::Protocol,
        format!("could not read the Grok billing snapshot: {error}"),
    )
}

fn protocol_failure(message: impl Into<String>) -> BackendFailure {
    BackendFailure::new(BackendFailureKind::Protocol, message)
}

#[cfg(test)]
mod tests;
