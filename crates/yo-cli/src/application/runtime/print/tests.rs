use super::finish_print_output;
use crate::diagnostic::AppError;

// print projection이 이미 framing한 bytes는 cleanup 성공 뒤 publisher에 정확히 한 번
// 전달되며 process layer가 두 번째 LF나 다른 stdout payload를 덧붙이지 않습니다.
#[test]
fn successful_cleanup_publishes_the_framed_answer_unchanged_once() {
    let mut calls = 0;
    let mut published = None;
    finish_print_output(Some("answer\n".to_owned()), Vec::new(), |output| {
        calls += 1;
        published = Some(output);
        Ok(())
    })
    .unwrap();

    assert_eq!(calls, 1);
    assert_eq!(published.as_deref(), Some("answer\n"));
}

// generation 또는 cleanup 실패가 하나라도 있으면 buffered answer의 stdout eligibility가
// 열리지 않아 publisher 자체를 호출하지 않습니다.
#[test]
fn failed_cleanup_keeps_buffered_output_unpublished() {
    let mut called = false;
    let error = finish_print_output(
        Some("ineligible\n".to_owned()),
        vec![AppError::message("cleanup failed")],
        |_| {
            called = true;
            Ok(())
        },
    )
    .unwrap_err();

    assert!(!called);
    assert!(error.to_string().contains("cleanup failed"));
}

// stdout publisher 자체의 실패도 성공으로 바뀌지 않으며 호출자가 만든 진단을 그대로
// 반환합니다.
#[test]
fn publication_failure_remains_a_process_failure() {
    let error = finish_print_output(Some("answer\n".to_owned()), Vec::new(), |_| {
        Err(AppError::message("stdout failed"))
    })
    .unwrap_err();

    assert!(error.to_string().contains("stdout failed"));
}
