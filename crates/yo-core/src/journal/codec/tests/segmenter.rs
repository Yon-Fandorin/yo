use std::time::Duration;

use super::{
    JournalCommit, JournalRecord, MessageOutcome, MessageSegment, MessageSegmenter, MessageStream,
    activity, encode, sequenced,
};

fn expect_segment(record: JournalRecord) -> MessageSegment {
    let JournalRecord::MessageSegment(segment) = record else {
        panic!("the boundary must emit a text segment");
    };
    segment
}

// 16KiB 경계에 걸친 다중 바이트 UTF-8 입력을 agent message로 나누어도 각 segment가
// 유효한 UTF-8 bound 안에 있고 다시 이으면 원문과 바이트 단위로 같아야 한다.
#[test]
fn splits_agent_text_at_valid_utf8_boundaries_without_changing_content() {
    let text = format!("{}한끝", "a".repeat((16 * 1024) - 1));
    let mut segmenter = MessageSegmenter::new(activity(), MessageStream::Agent);

    let mut segments = segmenter.push_text(&text, Duration::ZERO);
    if let Some(record) = segmenter.flush_boundary() {
        segments.push(expect_segment(record));
    }

    assert_eq!(
        segments
            .iter()
            .map(MessageSegment::text)
            .collect::<String>(),
        text
    );
    assert!(
        segments
            .iter()
            .all(|segment| segment.text().len() <= 16 * 1024)
    );
    assert_eq!(segments.len(), 2);
}

// 같은 크기의 text라도 tool output은 64KiB까지 한 segment로 유지되어 agent message와
// 서로 다른 계약 bound가 실제로 구분되는지 검증한다.
#[test]
fn uses_the_larger_bound_for_tool_output() {
    let text = "x".repeat(32 * 1024);
    let mut segmenter = MessageSegmenter::new(activity(), MessageStream::ToolOutput);

    assert!(segmenter.push_text(&text, Duration::ZERO).is_empty());
    let segment = expect_segment(
        segmenter
            .flush_boundary()
            .expect("a non-text boundary forces the buffered tool output"),
    );

    assert_eq!(segment.text(), text);
}

// 가장 오래된 미커밋 바이트가 정확히 1초에 도달하면 크기가 작아도 강제로 segment를
// 만들어 crash 시 무한정 큰 volatile tail이 남지 않게 해야 한다.
#[test]
fn flushes_a_small_tail_when_it_reaches_one_second() {
    let mut segmenter = MessageSegmenter::new(activity(), MessageStream::Agent);
    segmenter.push_text("tail", Duration::from_millis(10));

    assert!(segmenter.flush_due(Duration::from_millis(1009)).is_none());
    assert_eq!(
        expect_segment(
            segmenter
                .flush_due(Duration::from_millis(1010))
                .expect("the oldest byte reached one second"),
        )
        .text(),
        "tail"
    );
}

// message 종료 시 남은 tail과 MessageEnded가 한 record batch에 함께 나오고 seal의
// segment 수·UTF-8 byte 수가 전체 원문을 정확히 설명해야 불완전 복원을 탐지할 수 있다.
#[test]
fn seals_the_final_tail_with_exact_reconstruction_counts() {
    let mut segmenter = MessageSegmenter::new(activity(), MessageStream::Agent);
    segmenter.push_text("안녕", Duration::ZERO);

    let record = segmenter.finish(MessageOutcome::Completed);

    let JournalRecord::MessageEnded(terminal) = &record else {
        panic!("termination must produce one atomic terminal record");
    };
    let final_segment = terminal
        .final_segment()
        .expect("the non-empty final tail is embedded with its seal");
    let ended = terminal.ended();
    assert_eq!(final_segment.text(), "안녕");
    assert_eq!(ended.segment_count(), 1);
    assert_eq!(ended.utf8_bytes(), "안녕".len() as u64);
    assert_eq!(ended.outcome(), &MessageOutcome::Completed);
}

// codec가 선언된 stream bound보다 큰 직접 생성 segment를 허용하면 buffer를 우회한
// 손상 레코드가 durable storage에 들어갈 수 있으므로 encode 단계에서 거부해야 한다.
#[test]
fn rejects_an_oversized_segment_even_when_the_segmenter_is_bypassed() {
    let commit = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::MessageSegment(MessageSegment::new(
            activity(),
            MessageStream::Agent,
            1,
            "x".repeat((16 * 1024) + 1),
        ))],
    ));

    let error = encode(&commit).expect_err("admission enforces the semantic segment bound");

    assert!(error.to_string().contains("exceeds"));
}
