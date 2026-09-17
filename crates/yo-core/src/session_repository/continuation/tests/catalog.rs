use super::*;

// 완결된 Turn의 최신 Anchor는 해당 epoch의 versioned Codex locator와 결합되고,
// 다음 Turn ID와 기존 SubmissionId까지 한 번의 검증된 복구 결과에서 이어진다.
#[test]
fn derives_one_executable_plan_from_the_newest_durable_anchor() {
    let (_repository, continuation) = durable_resumable_session();

    assert_eq!(continuation.target().epoch(), 1);
    assert_eq!(
        continuation.target().binding().session_locator().value(),
        "thread-a"
    );
    assert_eq!(continuation.next_turn_id(), 2);
    assert_eq!(continuation.submission_ids().len(), 1);
    assert_eq!(
        continuation.target().model_replay().contract(),
        Some(&ModelReplayContract::new("system", Vec::new()))
    );
    assert_eq!(continuation.target().model_replay().items().len(), 2);
    assert_eq!(
        continuation.target().input_image_history(),
        InputImageHistory::TextOnly
    );
}
