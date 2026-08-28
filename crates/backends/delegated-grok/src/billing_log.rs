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
mod tests {
    use std::io::Write;

    use super::*;

    // 큰 unified log 전체를 읽지 않고 bounded tail의 최신 완전한 주간 관찰만 선택합니다.
    #[test]
    fn reads_the_newest_complete_weekly_snapshot_from_a_bounded_tail() {
        let path =
            std::env::temp_dir().join(format!("yo-grok-billing-{}.jsonl", std::process::id()));
        let mut file = File::create(&path).unwrap();
        writeln!(file, r#"{{"msg":"unrelated","ctx":{{}}}}"#).unwrap();
        writeln!(
            file,
            r#"{{"msg":"billing: fetched credits config","ctx":{{"config":{{"creditUsagePercent":12.1,"currentPeriod":{{"type":"USAGE_PERIOD_TYPE_WEEKLY","end":"2999-09-01T14:45:00Z"}}}}}}}}"#
        )
        .unwrap();

        let window = read_latest_usage(&path).unwrap().unwrap();
        std::fs::remove_file(path).unwrap();

        assert_eq!(window.used_percent(), 13);
        assert_eq!(window.used_percent_basis_points(), 1_210);
        assert_eq!(window.remaining_percent_basis_points(), 8_790);
        assert_eq!(window.window_duration_minutes(), Some(WEEKLY_MINUTES));
    }

    // proto3가 0을 생략하는 경우에도 유효한 current period가 있을 때만 0%로 해석합니다.
    #[test]
    fn treats_an_omitted_proto_zero_percentage_as_zero_only_with_a_valid_period() {
        let event = serde_json::json!({
            "msg": BILLING_MESSAGE,
            "ctx": { "config": { "currentPeriod": {
                "type": "USAGE_PERIOD_TYPE_WEEKLY",
                "end": "2999-09-01T14:45:00Z"
            }}}
        });

        assert_eq!(decode_usage(&event).unwrap().unwrap().used_percent(), 0);
        assert!(
            decode_usage(&serde_json::json!({ "msg": BILLING_MESSAGE }))
                .unwrap()
                .is_none()
        );
    }
}
