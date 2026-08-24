use std::fmt::Write as _;

#[cfg(test)]
use yo_core::TranscriptRecord;
use yo_core::{
    SessionId,
    session_repository::{SessionUsageError, SessionUsageProjection, UsageAggregate},
};

use super::ArchivedProjectionError;
use crate::{
    GlyphProfile,
    runner::usage_format::{aggregate_text, cache_read_text, safe_text, source_text, value_text},
};

pub(super) fn project(
    history: &yo_core::session_repository::StoredSessionHistory,
    glyph_profile: GlyphProfile,
) -> Result<String, ArchivedProjectionError> {
    let projection = history.session_usage().map_err(projection_error)?;
    Ok(render(
        history.descriptor().session_id(),
        &projection,
        glyph_profile,
    ))
}

#[cfg(test)]
fn project_records(
    session_id: SessionId,
    records: &[TranscriptRecord],
    glyph_profile: GlyphProfile,
) -> Result<String, ArchivedProjectionError> {
    let projection = SessionUsageProjection::from_records(records).map_err(projection_error)?;
    Ok(render(session_id, &projection, glyph_profile))
}

fn projection_error(error: SessionUsageError) -> ArchivedProjectionError {
    ArchivedProjectionError {
        detail: format!(
            "projecting stored Session Usage failed: invalid {} receipt for activity {:?}: {}",
            safe_text(error.schema()),
            error.activity(),
            safe_text(error.detail()),
        ),
    }
}

fn render(
    session_id: SessionId,
    projection: &SessionUsageProjection,
    glyph_profile: GlyphProfile,
) -> String {
    let receipt_count = projection.receipts().len();
    let mut output = format!(
        "Stored Session Usage\n\
         session={session_id}\n\
         completed_receipts={receipt_count}"
    );

    if receipt_count == 0 {
        output.push_str("\n\nNo completed usage receipts are available.");
        return output;
    }

    let aggregates = projection.aggregates();
    output.push_str("\n\nToken totals\n");
    push_aggregate_line(
        &mut output,
        "input",
        aggregates.input_tokens(),
        receipt_count,
    );
    push_aggregate_line(
        &mut output,
        "output",
        aggregates.output_tokens(),
        receipt_count,
    );
    push_aggregate_line(
        &mut output,
        "total",
        aggregates.total_tokens(),
        receipt_count,
    );
    push_aggregate_line(
        &mut output,
        "reasoning",
        aggregates.reasoning_tokens(),
        receipt_count,
    );

    let cache_read = projection.cache_read();
    output.push_str("\nCache read\ncache_read=");
    output.push_str(&cache_read_text(cache_read));

    output.push_str("\n\nReceipts (chronological)\n");
    let marker = receipt_marker(glyph_profile);
    for (index, receipt) in projection.receipts().iter().enumerate() {
        let _ = writeln!(
            output,
            "{marker} [{:02}] {}",
            index + 1,
            source_text(receipt, None)
        );
        let _ = writeln!(
            output,
            "  input={} output={} total={} reasoning={} cache_read={}",
            value_text(receipt.usage().input_tokens()),
            value_text(receipt.usage().output_tokens()),
            value_text(receipt.usage().total_tokens()),
            value_text(receipt.usage().reasoning_tokens()),
            value_text(receipt.usage().cache_read_input_tokens()),
        );
    }
    let _ = output.pop();
    output
}

fn push_aggregate_line(
    output: &mut String,
    name: &str,
    aggregate: UsageAggregate,
    receipt_count: usize,
) {
    let _ = writeln!(
        output,
        "{name}={}",
        aggregate_text(aggregate, receipt_count)
    );
}

const fn receipt_marker(profile: GlyphProfile) -> &'static str {
    match profile {
        GlyphProfile::Rich => "•",
        GlyphProfile::Ascii => "*",
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use yo_core::{
        ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentEvent,
        SessionId, TranscriptRecord, TurnId, TurnRef,
        session_repository::{CODEX_USAGE_SCHEMA, GROK_USAGE_SCHEMA, MANAGED_USAGE_SCHEMA},
    };

    use super::*;

    fn session_id() -> SessionId {
        "01890f00-0000-7000-8000-000000000001"
            .parse()
            .expect("fixture Session ID is valid")
    }

    fn receipt_records(receipts: &[(u64, String)]) -> Vec<TranscriptRecord> {
        receipts
            .iter()
            .flat_map(|(activity_id, text)| {
                let activity = activity(*activity_id);
                [
                    TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted {
                        activity,
                        kind: ActivityKind::ModelWork,
                    }),
                    TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
                        activity,
                        update: ActivityUpdate::TextSnapshot(text.clone()),
                    }),
                    TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished {
                        activity,
                        outcome: ActivityOutcome::Completed,
                    }),
                ]
            })
            .collect()
    }

    fn activity(activity_id: u64) -> ActivityRef {
        let turn = TurnRef::new(session_id(), TurnId::new(NonZeroU64::new(1).unwrap()));
        ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(activity_id).unwrap()))
    }

    fn managed(provider: &str, usage: &str, cache_read: &str) -> String {
        format!(
            r#"{{"schema":"{MANAGED_USAGE_SCHEMA}","response_id":"managed-response","round":1,"provider":"{provider}","account":"team","model":"kimi-k2","connector":"responses","api_dialect":"responses","base_url":"https://managed.invalid","usage":{usage},"cache_read_input_tokens":{cache_read}}}"#
        )
    }

    fn grok(usage: &str) -> String {
        format!(
            r#"{{"schema":"{GROK_USAGE_SCHEMA}","source_profile":"grok.acp.profile/v1","prompt_request_id":7,"usage":{usage}}}"#
        )
    }

    fn codex(usage: &str) -> String {
        format!(
            r#"{{"schema":"{CODEX_USAGE_SCHEMA}","source_profile":"codex.app.profile/v1","turn_id":"turn-1","model_context_window":32768,"usage":{usage},"thread_total":{{"input_tokens":100000,"output_tokens":100000,"total_tokens":200000,"reasoning_tokens":100000,"cache_read_input_tokens":100000,"cache_write_input_tokens":100000}}}}"#
        )
    }

    // 완전한 다중 제공자 영수증은 집계된 값과 관리형·Grok·Codex의 실제 출처를 순서대로 보여 주고,
    // 완전한 토큰 합계에는 불필요한 x/y를 붙이지 않는다.
    #[test]
    fn complete_usage_is_compact_and_chronological() {
        let receipts = vec![
            (
                1,
                managed(
                    r#"kimi\u001b"#,
                    r#"{"input_tokens":1000,"output_tokens":200,"total_tokens":1200,"reasoning_tokens":50}"#,
                    r#"{"availability":"reported","tokens":800,"source_profile":"managed.cache/v1"}"#,
                ),
            ),
            (
                2,
                grok(
                    r#"{"input_tokens":300,"output_tokens":100,"total_tokens":400,"reasoning_tokens":20,"cache_read_input_tokens":0,"cache_write_input_tokens":5}"#,
                ),
            ),
            (
                3,
                codex(
                    r#"{"input_tokens":500,"output_tokens":100,"total_tokens":600,"reasoning_tokens":0,"cache_read_input_tokens":100,"cache_write_input_tokens":10}"#,
                ),
            ),
        ];
        let output = project_records(
            session_id(),
            &receipt_records(&receipts),
            GlyphProfile::Rich,
        )
        .unwrap();

        assert!(output.starts_with("Stored Session Usage\nsession="));
        assert!(output.contains("completed_receipts=3"));
        assert!(output.contains("\ninput=1,800\n"));
        assert!(output.contains("\noutput=400\n"));
        assert!(output.contains("\ntotal=2,200\n"));
        assert!(output.contains("\nreasoning=70\n"));
        assert!(output.contains("cache_read=900/1,800 (50%) coverage=3/3"));
        assert!(!output.contains("coverage=3/3)"));
        assert!(output.contains("• [01] managed provider=kimi\\u{1b}"));
        assert!(output.contains("• [02] grok profile=grok.acp.profile/v1 request=7"));
        assert!(output.contains("• [03] codex profile=codex.app.profile/v1 turn=turn-1"));
        assert!(output.find("[01] managed").unwrap() < output.find("[02] grok").unwrap());
        assert!(output.find("[02] grok").unwrap() < output.find("[03] codex").unwrap());
        assert!(!output.contains("100,000"));
        assert!(
            output
                .lines()
                .all(|line| !line.chars().any(char::is_control))
        );
    }

    // ASCII 프로필은 Rich와 같은 의미와 순서를 유지하면서 기존 ASCII 마커만 사용해 파이프
    // 출력에서도 동일한 자료를 보장한다.
    #[test]
    fn ascii_usage_uses_the_existing_plain_marker() {
        let receipts = vec![(
            1,
            managed(
                "qwen",
                r#"{"input_tokens":1,"output_tokens":2,"total_tokens":3,"reasoning_tokens":0}"#,
                r#"{"availability":"reported","tokens":0,"source_profile":"managed.cache/v1"}"#,
            ),
        )];
        let output = project_records(
            session_id(),
            &receipt_records(&receipts),
            GlyphProfile::Ascii,
        )
        .unwrap();

        assert!(output.contains("* [01] managed provider=qwen"));
        assert!(!output.contains("• [01]"));
        assert!(!output.contains('\u{1b}'));
    }

    // 일부 토큰만 보고된 Session은 부분 합계와 독립적인 coverage를 표시하고,
    // zero·absent·unsupported cache-read 상태를 영수증별로 구별한다.
    #[test]
    fn partial_usage_exposes_independent_coverage_and_states() {
        let receipts = vec![
            (
                1,
                managed(
                    "kimi",
                    r#"{"input_tokens":100,"output_tokens":2,"total_tokens":102}"#,
                    r#"{"availability":"reported","tokens":0,"source_profile":"managed.cache/v1"}"#,
                ),
            ),
            (
                2,
                managed(
                    "qwen",
                    r#"{"input_tokens":50,"output_tokens":3,"total_tokens":53}"#,
                    r#"{"availability":"absent","source_profile":"managed.cache/v1"}"#,
                ),
            ),
            (
                3,
                managed(
                    "openai",
                    r#"{"output_tokens":4,"total_tokens":4}"#,
                    r#"{"availability":"unsupported"}"#,
                ),
            ),
        ];
        let output = project_records(
            session_id(),
            &receipt_records(&receipts),
            GlyphProfile::Rich,
        )
        .unwrap();

        assert!(output.contains("input=150 (coverage=2/3)"));
        assert!(output.contains("\noutput=9\n"));
        assert!(output.contains("\ntotal=159\n"));
        assert!(output.contains("reasoning=unavailable (coverage=0/3)"));
        assert!(output.contains("cache_read=0/100 (0%) coverage=1/3"));
        assert!(output.contains("cache_read=0\n"));
        assert!(output.contains("cache_read=absent\n"));
        assert!(output.contains("cache_read=unsupported"));
        assert!(output.contains("input=absent"));
    }

    // 영수증이 없는 오래된 Session은 성공적으로 명시적 빈 상태를 반환하며, 관측되지 않은 토큰을
    // 0으로 오해하게 만들지 않는다.
    #[test]
    fn empty_usage_is_a_successful_empty_report() {
        let output = project_records(session_id(), &[], GlyphProfile::Ascii).unwrap();

        assert!(output.contains("completed_receipts=0"));
        assert!(output.contains("No completed usage receipts are available."));
        assert!(!output.contains("Token totals"));
        assert!(!output.contains("input=0"));
    }

    // 알려진 schema를 주장한 malformed 영수증은 렌더링 전에 실패해 partial stdout이 생성될 경로를
    // 닫는다.
    #[test]
    fn malformed_known_receipt_fails_before_rendering() {
        let receipts = vec![(
            1,
            managed(
                "kimi",
                r#"{"input_tokens":"not-a-number"}"#,
                r#"{"availability":"reported","tokens":0,"source_profile":"managed.cache/v1"}"#,
            ),
        )];
        let error = project_records(
            session_id(),
            &receipt_records(&receipts),
            GlyphProfile::Rich,
        )
        .expect_err("known malformed receipts must fail closed");

        assert!(error.to_string().contains(MANAGED_USAGE_SCHEMA));
        assert!(error.to_string().contains("input_tokens"));
    }
}
