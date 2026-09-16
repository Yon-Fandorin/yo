use std::num::NonZeroU64;

use serde_json::Value;

use super::{
    projection::build_projection,
    receipts::{parse_codex, parse_grok_diagnostic, parse_managed, parse_receipt},
    *,
};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentEvent, TranscriptRecord,
};

// 알려진 사용량 영수증이 없는 기록은 성공하지만 모든 집계를 사용할 수 없음으로 남긴다.
#[test]
fn empty_history_has_unavailable_aggregates() {
    let projection = project_session_usage(&[]).unwrap();

    assert!(!projection.has_receipts());
    assert_eq!(
        projection.aggregates().input_tokens().coverage(),
        UsageCoverage::Unavailable
    );
    assert_eq!(
        projection.cache_read().coverage(),
        UsageCoverage::Unavailable
    );
}

// 관리형 영수증의 0, 선택적 부재, 제공자 미지원 상태를 서로 다른 값으로 보존한다.
#[test]
fn managed_receipt_preserves_zero_absent_and_unsupported() {
    let value = serde_json::json!({
        "schema": MANAGED_USAGE_SCHEMA,
        "response_id": "response-1",
        "round": 1,
        "provider": "managed",
        "account": "account-1",
        "model": "model-1",
        "connector": "connector-1",
        "api_dialect": "dialect-1",
        "base_url": "https://managed.invalid",
        "usage": {
            "input_tokens": 0,
            "output_tokens": 2,
            "total_tokens": 2,
            "reasoning_tokens": null
        },
        "cache_read_input_tokens": {
            "availability": "absent",
            "source_profile": "managed.cache-read/v1"
        }
    });
    let (source, usage) = parse_managed(&value).unwrap();
    let receipt = SessionUsageReceipt {
        activity: activity(1),
        source,
        usage,
    };

    assert_eq!(receipt.usage().input_tokens(), UsageValue::Reported(0));
    assert_eq!(receipt.usage().reasoning_tokens(), UsageValue::Absent);
    assert_eq!(
        receipt.usage().cache_read_input_tokens(),
        UsageValue::Absent
    );
    assert_eq!(
        receipt.usage().cache_write_input_tokens(),
        UsageValue::Unsupported
    );
}

// 관리형 Option token의 null과 누락은 absent로 보존하되 비정상 non-null은 거부한다.
#[test]
fn managed_optional_tokens_are_absent_but_invalid_values_fail() {
    let absent =
        serde_json::from_str::<Value>(&managed_text(None, None, None, None, "absent", 0)).unwrap();
    let (_, usage) = parse_managed(&absent).unwrap();
    assert_eq!(usage.input_tokens(), UsageValue::Absent);
    assert_eq!(usage.reasoning_tokens(), UsageValue::Absent);

    let mut malformed = serde_json::from_str::<Value>(&managed_text(
        Some(1),
        Some(1),
        Some(2),
        Some(0),
        "absent",
        0,
    ))
    .unwrap();
    malformed["usage"]["input_tokens"] = serde_json::json!("not a number");
    assert!(parse_managed(&malformed).is_err());
}

// 새 Grok diagnostic receipt는 whole-prompt token 값과 함께 host 내부 model-call 및
// turn 계수를 보존하고, 기존 v1 receipt는 같은 계수를 꾸며내지 않은 채 계속 읽습니다.
#[test]
fn grok_diagnostic_receipt_preserves_host_work_counts_without_reinterpreting_v1() {
    let diagnostic = serde_json::json!({
        "schema": GROK_USAGE_DIAGNOSTIC_SCHEMA,
        "source_profile": "grok.acp.prompt-response.meta-usage/v1",
        "prompt_request_id": 4,
        "model_calls": 7,
        "num_turns": 3,
        "usage": {
            "input_tokens": 990_347,
            "output_tokens": 17_988,
            "total_tokens": 1_008_335,
            "reasoning_tokens": 16_432,
            "cache_read_input_tokens": 585_728,
            "cache_write_input_tokens": 0
        }
    });
    let diagnostic = parse_receipt(&diagnostic.to_string(), activity(1))
        .unwrap()
        .unwrap();
    assert_eq!(diagnostic.schema(), GROK_USAGE_DIAGNOSTIC_SCHEMA);
    assert!(matches!(
        diagnostic.source(),
        SessionUsageSource::GrokDiagnostic {
            model_calls: 7,
            num_turns: 3,
            ..
        }
    ));

    let stable = parse_receipt(&grok_text(20), activity(2)).unwrap().unwrap();
    assert_eq!(stable.schema(), GROK_USAGE_SCHEMA);
    assert!(matches!(stable.source(), SessionUsageSource::Grok { .. }));
}

// experimental diagnostic shape는 호출 계수와 token 객체를 닫아 부분 관측이나
// 알 수 없는 필드가 분석 근거로 조용히 들어오지 못하게 합니다.
#[test]
fn grok_diagnostic_receipt_rejects_missing_or_extra_diagnostics() {
    let mut value = serde_json::json!({
        "schema": GROK_USAGE_DIAGNOSTIC_SCHEMA,
        "source_profile": "grok.acp.prompt-response.meta-usage/v1",
        "prompt_request_id": 4,
        "model_calls": 1,
        "num_turns": 1,
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1,
            "total_tokens": 2,
            "reasoning_tokens": 0,
            "cache_read_input_tokens": 0,
            "cache_write_input_tokens": 0
        }
    });
    value.as_object_mut().unwrap().remove("model_calls");
    assert!(parse_grok_diagnostic(&value).is_err());

    value["model_calls"] = serde_json::json!(1);
    value["usage"]["unexpected"] = serde_json::json!(true);
    assert!(parse_grok_diagnostic(&value).is_err());
}

// managed cache availability는 reported·absent·unsupported마다 허용 필드와 versioned
// source profile이 다르므로 누락·초과·잘못된 profile을 모두 malformed로 닫습니다.
#[test]
fn managed_cache_availability_shape_is_closed() {
    let malformed = [
        serde_json::json!({"availability": "reported", "tokens": 1}),
        serde_json::json!({
            "availability": "reported",
            "tokens": 1,
            "source_profile": "managed.cache/v1",
            "extra": true
        }),
        serde_json::json!({
            "availability": "absent",
            "source_profile": "managed.cache/v1",
            "tokens": 1
        }),
        serde_json::json!({"availability": "absent"}),
        serde_json::json!({
            "availability": "unsupported",
            "source_profile": "managed.cache/v1"
        }),
        serde_json::json!({"availability": "unsupported", "tokens": 1}),
        serde_json::json!({
            "availability": "reported",
            "tokens": 1,
            "source_profile": "not-versioned"
        }),
    ];

    for cache in malformed {
        let mut receipt = serde_json::from_str::<Value>(&managed_text(
            Some(1),
            Some(1),
            Some(2),
            Some(0),
            "reported",
            0,
        ))
        .unwrap();
        receipt["cache_read_input_tokens"] = cache;
        assert!(parse_managed(&receipt).is_err(), "{receipt}");
    }
}

// recognized managed receipt의 root와 usage는 닫힌 형식이므로 알려지지 않은 필드를
// 조용히 버리지 않고 전체 projection 오류로 되돌릴 parse 오류를 만듭니다.
#[test]
fn managed_receipt_rejects_unknown_root_and_usage_fields() {
    let mut root = serde_json::from_str::<Value>(&managed_text(
        Some(1),
        Some(1),
        Some(2),
        Some(0),
        "reported",
        0,
    ))
    .unwrap();
    root["unexpected"] = serde_json::json!(true);
    assert!(parse_managed(&root).is_err());

    let mut usage = serde_json::from_str::<Value>(&managed_text(
        Some(1),
        Some(1),
        Some(2),
        Some(0),
        "reported",
        0,
    ))
    .unwrap();
    usage["usage"]["unexpected"] = serde_json::json!(true);
    assert!(parse_managed(&usage).is_err());
}

// Codex 집계는 현재 턴 usage만 사용하고 누적 thread_total은 합산하지 않는다.
#[test]
fn codex_ignores_thread_total() {
    let value = serde_json::json!({
        "schema": CODEX_USAGE_SCHEMA,
        "source_profile": "codex.app-server.thread-token-usage-updated/v1",
        "turn_id": "turn-1",
        "model_context_window": 8192,
        "usage": {
            "input_tokens": 3,
            "output_tokens": 4,
            "total_tokens": 7,
            "reasoning_tokens": 0,
            "cache_read_input_tokens": 1,
            "cache_write_input_tokens": 2
        },
        "thread_total": {
            "input_tokens": 300,
            "output_tokens": 400,
            "total_tokens": 700,
            "reasoning_tokens": 200,
            "cache_read_input_tokens": 100,
            "cache_write_input_tokens": 20
        }
    });
    let (source, usage) = parse_codex(&value).unwrap();
    let receipt = SessionUsageReceipt {
        activity: activity(2),
        source,
        usage,
    };

    assert_eq!(receipt.usage().input_tokens(), UsageValue::Reported(3));
    assert_eq!(receipt.usage().output_tokens(), UsageValue::Reported(4));
    assert_eq!(
        receipt.usage().cache_read_input_tokens(),
        UsageValue::Reported(1)
    );
}

// 일부 영수증만 필드를 보고하면 합계와 함께 부분 x/y 커버리지를 표시한다.
#[test]
fn aggregates_report_coverage_without_hiding_partial_sum() {
    let first = SessionUsageReceipt {
        activity: activity(1),
        source: managed_source(),
        usage: SessionUsage {
            input_tokens: UsageValue::Reported(2),
            output_tokens: UsageValue::Reported(3),
            total_tokens: UsageValue::Reported(5),
            reasoning_tokens: UsageValue::Absent,
            cache_read_input_tokens: UsageValue::Reported(0),
            cache_write_input_tokens: UsageValue::Unsupported,
        },
    };
    let second = SessionUsageReceipt {
        activity: activity(2),
        source: grok_source(),
        usage: SessionUsage {
            input_tokens: UsageValue::Reported(4),
            output_tokens: UsageValue::Reported(1),
            total_tokens: UsageValue::Reported(5),
            reasoning_tokens: UsageValue::Reported(1),
            cache_read_input_tokens: UsageValue::Absent,
            cache_write_input_tokens: UsageValue::Reported(2),
        },
    };
    let projection = build_projection(vec![first, second]).unwrap();

    assert_eq!(projection.aggregates().input_tokens().tokens(), Some(6));
    assert_eq!(
        projection.aggregates().reasoning_tokens().coverage(),
        UsageCoverage::Partial {
            reported: 1,
            total: 2
        }
    );
    assert_eq!(projection.cache_read().eligible_receipts(), 1);
    assert_eq!(projection.cache_read().total_receipts(), 2);
}

// 토큰 합계가 u64 범위를 넘으면 포화시키지 않고 typed overflow error로 닫는다.
#[test]
fn aggregation_overflow_is_reported_as_an_error() {
    let mut first_usage = SessionUsage {
        input_tokens: UsageValue::Reported(u64::MAX),
        output_tokens: UsageValue::Reported(0),
        total_tokens: UsageValue::Reported(0),
        reasoning_tokens: UsageValue::Unsupported,
        cache_read_input_tokens: UsageValue::Unsupported,
        cache_write_input_tokens: UsageValue::Unsupported,
    };
    let first = SessionUsageReceipt {
        activity: activity(1),
        source: managed_source(),
        usage: first_usage.clone(),
    };
    first_usage.input_tokens = UsageValue::Reported(1);
    let second = SessionUsageReceipt {
        activity: activity(2),
        source: managed_source(),
        usage: first_usage,
    };

    let error = build_projection(vec![first, second]).unwrap_err();

    assert_eq!(error.activity(), activity(2));
    assert_eq!(error.schema(), MANAGED_USAGE_SCHEMA);
    assert!(error.detail().contains("input_tokens"));
}

// 세 제공자의 완료된 ModelWork 수명주기를 ActivityRef와 영수증 순서대로 보존한다.
#[test]
fn lifecycle_projection_preserves_sources_activity_and_chronology() {
    let mut records = completed(
        activity(1),
        managed_text(Some(10), Some(2), Some(12), None, "reported", 0),
    );
    records.extend(completed(activity(2), grok_text(20)));
    records.extend(completed(activity(3), codex_text(30)));

    let projection = project_session_usage(&records).unwrap();

    assert_eq!(projection.receipts().len(), 3);
    assert_eq!(projection.receipts()[0].activity(), activity(1));
    assert_eq!(
        projection.receipts()[0].provider(),
        SessionUsageProvider::Managed
    );
    assert_eq!(projection.receipts()[0].source(), &managed_source());
    assert_eq!(projection.receipts()[1].activity(), activity(2));
    assert_eq!(
        projection.receipts()[1].provider(),
        SessionUsageProvider::Grok
    );
    assert_eq!(projection.receipts()[1].source(), &grok_source());
    assert_eq!(projection.receipts()[2].activity(), activity(3));
    assert_eq!(
        projection.receipts()[2].provider(),
        SessionUsageProvider::Codex
    );
    assert_eq!(projection.receipts()[2].source(), &codex_source());
}

// 미완료, 실패, 비영수증, 마지막 snapshot이 아닌 ModelWork는 집계하지 않는다.
#[test]
fn lifecycle_projection_ignores_non_completed_or_non_receipt_work() {
    let mut records = unfinished(
        activity(1),
        managed_text(Some(1), Some(1), Some(2), Some(0), "reported", 0),
    );
    records.extend(failed(activity(2), grok_text(2)));
    records.extend(completed(activity(3), "not a receipt".to_owned()));
    records.extend(completed_with_delta_after_snapshot(
        activity(4),
        codex_text(3),
    ));

    let projection = project_session_usage(&records).unwrap();

    assert!(!projection.has_receipts());
}

// 알려진 schema의 완료된 영수증 구조가 깨지면 부분 결과 대신 typed error를 반환한다.
#[test]
fn lifecycle_projection_rejects_malformed_known_schema() {
    let malformed = serde_json::json!({
        "schema": CODEX_USAGE_SCHEMA,
        "source_profile": "codex.app-server.thread-token-usage-updated/v1",
        "turn_id": "turn-1",
        "usage": {
            "input_tokens": 1,
            "output_tokens": 2
        }
    })
    .to_string();

    let error = project_session_usage(&completed(activity(1), malformed)).unwrap_err();

    assert_eq!(error.schema(), CODEX_USAGE_SCHEMA);
    assert!(error.detail().contains("total_tokens"));
}

// cache-read의 보고된 0, absent, unsupported를 보존하고 eligible 커버리지를 부분으로 표시한다.
#[test]
fn cache_read_zero_absent_and_unsupported_have_partial_coverage() {
    let mut records = completed(
        activity(1),
        managed_text(Some(10), Some(1), Some(11), Some(0), "reported", 0),
    );
    records.extend(completed(
        activity(2),
        managed_text(Some(20), Some(2), Some(22), Some(0), "absent", 0),
    ));
    records.extend(completed(
        activity(3),
        managed_text(Some(30), Some(3), Some(33), Some(0), "unsupported", 0),
    ));

    let projection = project_session_usage(&records).unwrap();

    assert_eq!(
        projection.receipts()[0].usage().cache_read_input_tokens(),
        UsageValue::Reported(0)
    );
    assert_eq!(
        projection.receipts()[1].usage().cache_read_input_tokens(),
        UsageValue::Absent
    );
    assert_eq!(
        projection.receipts()[2].usage().cache_read_input_tokens(),
        UsageValue::Unsupported
    );
    assert_eq!(
        projection.cache_read().coverage(),
        UsageCoverage::Partial {
            reported: 1,
            total: 3
        }
    );
    assert_eq!(
        projection.cache_read().share(),
        Some(CacheReadShare {
            cache_read_tokens: 0,
            input_tokens: 10,
        })
    );
}

fn activity(number: u64) -> ActivityRef {
    let session_id: crate::SessionId = "018f0a00-0000-7000-8000-000000000001".parse().unwrap();
    let turn = crate::TurnRef::new(
        session_id,
        crate::TurnId::new(NonZeroU64::new(number).unwrap()),
    );
    ActivityRef::new(
        turn,
        crate::ActivityId::new(NonZeroU64::new(number).unwrap()),
    )
}

fn managed_source() -> SessionUsageSource {
    SessionUsageSource::Managed {
        response_id: "response-1".to_owned(),
        round: 1,
        provider: "managed".to_owned(),
        account: "account-1".to_owned(),
        model: "model-1".to_owned(),
        connector: "connector-1".to_owned(),
        api_dialect: "dialect-1".to_owned(),
        base_url: "https://managed.invalid".to_owned(),
    }
}

fn grok_source() -> SessionUsageSource {
    SessionUsageSource::Grok {
        source_profile: "grok.acp.prompt-response.usage/v1".to_owned(),
        prompt_request_id: 42,
    }
}

fn codex_source() -> SessionUsageSource {
    SessionUsageSource::Codex {
        source_profile: "codex.app-server.thread-token-usage-updated/v1".to_owned(),
        turn_id: "turn-1".to_owned(),
        model_context_window: Some(8192),
    }
}

fn managed_text(
    input: Option<u64>,
    output: Option<u64>,
    total: Option<u64>,
    reasoning: Option<u64>,
    cache_availability: &str,
    cache_tokens: u64,
) -> String {
    let cache_read_input_tokens = match cache_availability {
        "reported" => serde_json::json!({
            "availability": "reported",
            "tokens": cache_tokens,
            "source_profile": "managed.cache-read/v1"
        }),
        "absent" => serde_json::json!({
            "availability": "absent",
            "source_profile": "managed.cache-read/v1"
        }),
        "unsupported" => serde_json::json!({
            "availability": "unsupported"
        }),
        availability => serde_json::json!({
            "availability": availability
        }),
    };
    serde_json::json!({
        "schema": MANAGED_USAGE_SCHEMA,
        "response_id": "response-1",
        "round": 1,
        "provider": "managed",
        "account": "account-1",
        "model": "model-1",
        "connector": "connector-1",
        "api_dialect": "dialect-1",
        "base_url": "https://managed.invalid",
        "usage": {
            "input_tokens": input,
            "output_tokens": output,
            "total_tokens": total,
            "reasoning_tokens": reasoning
        },
        "cache_read_input_tokens": cache_read_input_tokens
    })
    .to_string()
}

fn grok_text(input: u64) -> String {
    serde_json::json!({
        "schema": GROK_USAGE_SCHEMA,
        "source_profile": "grok.acp.prompt-response.usage/v1",
        "prompt_request_id": 42,
        "usage": {
            "input_tokens": input,
            "output_tokens": 2,
            "total_tokens": input + 2,
            "reasoning_tokens": 0,
            "cache_read_input_tokens": 0,
            "cache_write_input_tokens": 0
        }
    })
    .to_string()
}

fn codex_text(input: u64) -> String {
    serde_json::json!({
        "schema": CODEX_USAGE_SCHEMA,
        "source_profile": "codex.app-server.thread-token-usage-updated/v1",
        "turn_id": "turn-1",
        "model_context_window": 8192,
        "usage": {
            "input_tokens": input,
            "output_tokens": 2,
            "total_tokens": input + 2,
            "reasoning_tokens": 0,
            "cache_read_input_tokens": 1,
            "cache_write_input_tokens": 0
        },
        "thread_total": {
            "input_tokens": 900,
            "output_tokens": 900,
            "total_tokens": 1800,
            "reasoning_tokens": 900,
            "cache_read_input_tokens": 900,
            "cache_write_input_tokens": 900
        }
    })
    .to_string()
}

fn completed(activity: ActivityRef, text: String) -> Vec<TranscriptRecord> {
    vec![
        TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ModelWork,
        }),
        TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(text),
        }),
        TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        }),
    ]
}

fn unfinished(activity: ActivityRef, text: String) -> Vec<TranscriptRecord> {
    completed(activity, text)[..2].to_vec()
}

fn failed(activity: ActivityRef, text: String) -> Vec<TranscriptRecord> {
    let mut records = completed(activity, text);
    records.pop();
    records.push(TranscriptRecord::EventCommitted(
        AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Failed(crate::Failure::new("failed")),
        },
    ));
    records
}

fn completed_with_delta_after_snapshot(
    activity: ActivityRef,
    text: String,
) -> Vec<TranscriptRecord> {
    let mut records = completed(activity, text);
    records.insert(
        records.len() - 1,
        TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextDelta("trailing delta".to_owned()),
        }),
    );
    records
}
