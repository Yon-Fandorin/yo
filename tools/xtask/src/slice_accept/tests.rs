use super::compose_message;

// 게이트가 소유하는 review trailer만 기계적으로 덧붙이고 사람이 작성한 의미 설명과
// Developer Docs 판단은 그대로 보존한다.
#[test]
fn commit_message_appends_exact_gate_trailers() {
    let message = compose_message(
        b"feat: accept Slice\n\nExplain the accepted effect.\n\nDeveloper-Docs-Impact: updated\n",
        &[
            "Slice-Review: fresh-context - completed - codex/test - clear".to_owned(),
            "Review-Coverage: fresh-context - exact - model-high/codex/test - sha256:abc"
                .to_owned(),
        ],
    )
    .unwrap();

    assert_eq!(
        String::from_utf8(message).unwrap(),
        "feat: accept Slice\n\nExplain the accepted effect.\n\nDeveloper-Docs-Impact: updated\n\nSlice-Review: fresh-context - completed - codex/test - clear\nReview-Coverage: fresh-context - exact - model-high/codex/test - sha256:abc\n"
    );
}

// 사람이 준비한 원문에 이전 후보의 review trailer가 섞이면 ready 게이트의 exact
// trailer와 공존시키지 않고 입력 경계에서 바로 거부한다.
#[test]
fn commit_message_rejects_caller_review_trailers() {
    for trailer in ["Slice-Review:", "Review-Coverage:"] {
        let source = format!("feat: stale\n\n{trailer} old\n");
        let error = compose_message(
            source.as_bytes(),
            &["Slice-Review: none - exact".to_owned()],
        )
        .unwrap_err();
        assert!(error.contains("must omit gate-derived review trailers"));
    }
}
