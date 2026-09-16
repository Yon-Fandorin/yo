use super::*;

// 외부 backend가 전달한 제어 문자는 status line의 행 구조를 바꾸지 못하고 보이는 표기로 바뀐다.
#[test]
fn session_info_escapes_control_characters_into_one_line() {
    let info = TuiSessionInfo::new("co\ndex", "work\tspace");

    assert_eq!(info.backend(), Some("co\\ndex"));
    assert_eq!(info.workspace(), "work\\tspace");
}
// 시작 안내는 opt-in이며 재개 여부·원문 제어 표기와 표시 한도의 정확한 경계를 보존한다.
#[test]
fn startup_notice_uses_confirmed_origin_and_bounded_labels() {
    assert!(
        TuiSessionInfo::new("native", "~/yo")
            .startup_notice()
            .is_none()
    );
    assert!(
        TuiSessionInfo::default()
            .with_startup_notice(false)
            .startup_notice()
            .is_none()
    );
    for resumed in [false, true] {
        let notice = TuiSessionInfo::new("co\ndex", "~/yo")
            .with_startup_notice(resumed)
            .startup_notice()
            .unwrap();
        assert_eq!(
            notice.title,
            if resumed {
                "Session resumed"
            } else {
                "New session"
            }
        );
        assert_eq!(notice.message, "Backend: co\\ndex\nWorkspace: ~/yo");
        assert_eq!(notice.level, NoticeLevel::Info);
    }
    for length in [4096, 4097] {
        let notice = TuiSessionInfo::new("가".repeat(length), "~/yo")
            .with_startup_notice(false)
            .startup_notice()
            .unwrap();
        assert_eq!(notice.message.matches('가').count(), 4096);
        assert_eq!(notice.message.contains('…'), length == 4097);
        assert!(notice.to_snapshot().is_some());
    }
}
// 상태 snapshot의 키·개수·escaped text 한도를 검증하고 실패한 입력은 display 값을 만들지
// 않는다.
#[test]
fn host_status_snapshot_is_bounded_ordered_and_control_safe() {
    use super::{TuiStatusError, TuiStatusLine};
    let status = TuiStatusLine::new([("z", "last"), ("a", "first\n\x1b[31m")]).unwrap();
    assert_eq!(status.as_str(), "first\\n\\u{1b}[31m · last");
    for count in [16, 17] {
        let result = TuiStatusLine::new((0..count).map(|index| (index.to_string(), "x")));
        assert_eq!(result.is_ok(), count == 16);
    }
    for len in [64, 65] {
        assert_eq!(
            TuiStatusLine::new([("k".repeat(len), "value")]).is_ok(),
            len == 64
        );
    }
    for len in [1024, 1025] {
        assert_eq!(
            TuiStatusLine::new([("key", "x".repeat(len))]).is_ok(),
            len == 1024
        );
    }
    assert!(TuiStatusLine::new([("key", "\n".repeat(512))]).is_ok());
    assert_eq!(
        TuiStatusLine::new([("key", "\n".repeat(513))]),
        Err(TuiStatusError::TextTooLong)
    );
    assert_eq!(
        TuiStatusLine::new([("same", "a"), ("same", "b")]),
        Err(TuiStatusError::DuplicateKey)
    );
    for key in ["", "bad\nkey"] {
        assert_eq!(
            TuiStatusLine::new([(key, "value")]),
            Err(TuiStatusError::InvalidKey)
        );
    }
    assert_eq!(
        TuiStatusLine::new([("empty", "")]).unwrap(),
        TuiStatusLine::default()
    );
}
