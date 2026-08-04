use super::id;
use crate::transcript::{TranscriptLayoutConfig, TranscriptState};

// 일반 텍스트 출력도 화면과 같은 marker·들여쓰기·턴 간격을 사용해 종료 뒤 대화 맥락을 보존한다.
#[test]
fn plain_output_preserves_the_transcript_projection() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "가\n나".to_owned())
        .expect("unique user item");
    transcript.start_assistant(id(2)).expect("unique assistant");
    transcript
        .append_text(id(2), "답")
        .expect("streaming assistant");

    let output = transcript
        .plain_output(&TranscriptLayoutConfig::default())
        .unwrap();

    assert_eq!(output.as_deref(), Some("❯ 가\n  나\n\n• 답\n"));
}

// 빈 streaming 항목만 있으면 종료 뒤 의미 없는 marker나 빈 줄을 출력하지 않는다.
#[test]
fn empty_streaming_transcript_has_no_plain_output() {
    let mut transcript = TranscriptState::new();
    transcript.start_assistant(id(1)).expect("unique assistant");

    let output = transcript
        .plain_output(&TranscriptLayoutConfig::default())
        .unwrap();

    assert_eq!(output, None);
}

// terminal control 문자는 원문 byte로 재실행하지 않고 화면과 같은 가시 표기로 안전하게 남긴다.
#[test]
fn plain_output_projects_control_characters_safely() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "before\u{1b}after".to_owned())
        .expect("unique user item");

    let output = transcript
        .plain_output(&TranscriptLayoutConfig::default())
        .unwrap();

    assert_eq!(output.as_deref(), Some("❯ before^[after\n"));
}
