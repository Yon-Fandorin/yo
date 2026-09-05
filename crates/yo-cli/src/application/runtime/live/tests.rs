use super::*;

// 저장소가 아직 없는 머신에서 명시적 resume를 요청하면 내부 상태 오류 대신 요청한
// Session ID가 없다는 기존 진단을 반환하고, 출력할 history를 만들지 않습니다.
#[test]
fn missing_read_only_storage_reports_requested_session_not_found() {
    let session_id = "01890f00-0000-7000-8000-000000000001".parse().unwrap();

    let result = read_only_resume_output(
        None,
        session_id,
        yo_tui::GlyphProfile::Rich,
        "continuation is unavailable",
    );

    let Err(error) = result else {
        panic!("an absent repository cannot provide stored Session output");
    };
    assert_eq!(
        error.to_string(),
        format!("stored Session {session_id} was not found")
    );
}
