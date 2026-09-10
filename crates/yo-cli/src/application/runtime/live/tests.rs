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

// 저장 세션 선택 시 현재 모델·도구·sandbox 강제 설정은 넘기지 않고 화면 설정만 유지한다.
#[test]
fn saved_session_options_preserve_presentation_without_overriding_saved_execution() {
    let target = "01890f00-0000-7000-8000-000000000009".parse().unwrap();
    let options = command::LiveOptions {
        mode: yo_tui::PresentationMode::Fullscreen,
        theme: None,
        glyph_profile: yo_tui::GlyphProfile::Ascii,
        selection: command::LiveSelection::New,
        model: Some("host:codex".to_owned()),
        no_tools: true,
        sandbox: Some(command::SandboxMode::ReadOnly),
    };
    for selection in [
        command::LiveSelection::Resume(target),
        command::LiveSelection::New,
    ] {
        let selected = saved_execution_options(options.clone(), selection);
        assert_eq!(selected.selection, selection);
        assert!(selected.model.is_none());
        assert!(!selected.no_tools);
        assert!(selected.sandbox.is_none());
        assert_eq!(selected.mode, options.mode);
        assert_eq!(selected.theme, options.theme);
        assert_eq!(selected.glyph_profile, options.glyph_profile);
    }
}
