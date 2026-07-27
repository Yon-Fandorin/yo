use super::{TranscriptPhase, TranscriptState, TranscriptStateError, id, message};

// 이미 사용 중인 ID는 기존 순서와 내용을 바꾸지 않고 거절한다.
#[test]
fn duplicate_id_preserves_existing_state() {
    let mut transcript = TranscriptState::new();
    transcript.push_user(id(1), "original".into()).unwrap();
    let before = transcript.clone();

    let error = transcript.start_assistant(id(1)).unwrap_err();

    assert_eq!(error, TranscriptStateError::DuplicateId(id(1)));
    assert_eq!(transcript, before);
}

// 존재하지 않는 ID의 append와 Final 요청은 어떤 항목도 만들지 않는다.
#[test]
fn unknown_id_updates_preserve_existing_state() {
    let mut transcript = TranscriptState::new();
    transcript.push_user(id(1), "known".into()).unwrap();
    let before = transcript.clone();

    let append_error = transcript.append_text(id(2), "lost").unwrap_err();
    let final_error = transcript.finalize(id(2)).unwrap_err();

    assert_eq!(append_error, TranscriptStateError::UnknownId(id(2)));
    assert_eq!(final_error, TranscriptStateError::UnknownId(id(2)));
    assert_eq!(transcript, before);
}

// Final 항목은 append와 중복 Final 모두 거절하며 revision과 텍스트를 보존한다.
#[test]
fn final_item_rejects_further_mutation() {
    let mut transcript = TranscriptState::new();
    transcript.start_assistant(id(1)).unwrap();
    transcript.append_text(id(1), "complete").unwrap();
    transcript.finalize(id(1)).unwrap();
    let before = transcript.clone();

    let append_error = transcript.append_text(id(1), "!").unwrap_err();
    let final_error = transcript.finalize(id(1)).unwrap_err();

    assert_eq!(append_error, TranscriptStateError::FinalItem(id(1)));
    assert_eq!(final_error, TranscriptStateError::FinalItem(id(1)));
    assert_eq!(transcript, before);
}

// revision 최대값에서는 append가 텍스트를 붙이기 전에 원자적으로 실패한다.
#[test]
fn append_revision_overflow_preserves_text_and_phase() {
    let mut transcript = TranscriptState::new();
    transcript.start_assistant(id(1)).unwrap();
    transcript.items[0].revision = u64::MAX;
    let before = transcript.clone();

    let error = transcript.append_text(id(1), "lost").unwrap_err();

    assert_eq!(error, TranscriptStateError::RevisionOverflow(id(1)));
    assert_eq!(transcript, before);
}

// revision 최대값에서는 Final 상태를 쓰기 전에 원자적으로 실패한다.
#[test]
fn final_revision_overflow_preserves_streaming_item() {
    let mut transcript = TranscriptState::new();
    transcript.start_assistant(id(1)).unwrap();
    transcript.items[0].revision = u64::MAX;
    let before = transcript.clone();

    let error = transcript.finalize(id(1)).unwrap_err();

    assert_eq!(error, TranscriptStateError::RevisionOverflow(id(1)));
    assert_eq!(transcript, before);
    assert_eq!(transcript.items()[0].phase(), TranscriptPhase::Streaming);
    assert_eq!(message(&transcript.items()[0]).text(), "");
}
