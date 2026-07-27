use super::{MessageRole, TranscriptPhase, TranscriptState, id, message};

// assistant 항목은 같은 ID를 유지한 채 append마다 revision을 한 번씩 증가시킨다.
#[test]
fn appends_streaming_assistant_text_with_monotonic_revisions() {
    let mut transcript = TranscriptState::new();
    transcript.start_assistant(id(7)).unwrap();

    transcript.append_text(id(7), "검").unwrap();
    transcript.append_text(id(7), "토 중").unwrap();

    let item = &transcript.items()[0];
    assert_eq!(item.id(), id(7));
    assert_eq!(item.revision(), 2);
    assert_eq!(item.phase(), TranscriptPhase::Streaming);
    assert_eq!(message(item).role(), MessageRole::Assistant);
    assert_eq!(message(item).text(), "검토 중");
}

// 여러 항목 중 선택한 ID만 갱신하고 인접한 항목의 순서와 내용은 건드리지 않는다.
#[test]
fn updates_the_nonzero_index_selected_by_id() {
    let mut transcript = TranscriptState::new();
    transcript.push_user(id(10), "keep".into()).unwrap();
    transcript.start_assistant(id(20)).unwrap();

    transcript.append_text(id(20), "target").unwrap();
    transcript.finalize(id(20)).unwrap();

    assert_eq!(transcript.items()[0].id(), id(10));
    assert_eq!(transcript.items()[0].revision(), 0);
    assert_eq!(transcript.items()[0].phase(), TranscriptPhase::Final);
    assert_eq!(message(&transcript.items()[0]).text(), "keep");
    assert_eq!(transcript.items()[1].id(), id(20));
    assert_eq!(transcript.items()[1].revision(), 2);
    assert_eq!(transcript.items()[1].phase(), TranscriptPhase::Final);
    assert_eq!(message(&transcript.items()[1]).text(), "target");
}

// 빈 streaming 조각은 표시 상태를 바꾸지 않으므로 revision도 증가시키지 않는다.
#[test]
fn empty_streaming_delta_is_unchanged() {
    let mut transcript = TranscriptState::new();
    transcript.start_assistant(id(7)).unwrap();
    let before = transcript.clone();

    transcript.append_text(id(7), "").unwrap();

    assert_eq!(transcript, before);
}

// revision 최대값에서도 빈 조각은 overflow를 검사할 필요 없는 완전한 no-op이다.
#[test]
fn empty_streaming_delta_is_unchanged_at_maximum_revision() {
    let mut transcript = TranscriptState::new();
    transcript.start_assistant(id(7)).unwrap();
    transcript.items[0].revision = u64::MAX;
    let before = transcript.clone();

    transcript.append_text(id(7), "").unwrap();

    assert_eq!(transcript, before);
}

// Final 전이는 revision을 증가시키고 그 시점의 완성된 텍스트를 잠근다.
#[test]
fn finalization_advances_revision_and_locks_text() {
    let mut transcript = TranscriptState::new();
    transcript.start_assistant(id(7)).unwrap();
    transcript.append_text(id(7), "done").unwrap();

    transcript.finalize(id(7)).unwrap();

    let item = &transcript.items()[0];
    assert_eq!(item.revision(), 2);
    assert_eq!(item.phase(), TranscriptPhase::Final);
    assert_eq!(message(item).text(), "done");
}
