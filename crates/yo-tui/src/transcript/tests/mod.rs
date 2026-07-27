use super::{
    MessageRole, TranscriptBody, TranscriptItem, TranscriptItemId, TranscriptPhase,
    TranscriptState, TranscriptStateError,
};

mod failures;
mod streaming;

fn id(value: u64) -> TranscriptItemId {
    TranscriptItemId::new(value)
}

fn message(item: &TranscriptItem) -> &super::TranscriptMessage {
    let TranscriptBody::Message(message) = item.body();
    message
}

// user 메시지는 전달 순서를 유지하며 처음부터 변경 불가능한 Final 항목이다.
#[test]
fn stores_user_messages_as_ordered_final_items() {
    let mut transcript = TranscriptState::new();

    transcript.push_user(id(10), "first".into()).unwrap();
    transcript.push_user(id(20), "second".into()).unwrap();

    assert_eq!(
        transcript
            .items()
            .iter()
            .map(TranscriptItem::id)
            .collect::<Vec<_>>(),
        [id(10), id(20)]
    );
    assert_eq!(transcript.items()[0].revision(), 0);
    assert_eq!(transcript.items()[0].phase(), TranscriptPhase::Final);
    assert_eq!(message(&transcript.items()[0]).role(), MessageRole::User);
    assert_eq!(message(&transcript.items()[0]).text(), "first");
}
