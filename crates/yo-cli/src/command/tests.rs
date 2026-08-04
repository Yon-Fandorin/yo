use super::*;

// 인자가 없으면 기존 제품 진입점인 live Inline/Rich 실행으로 남아 `session` 기능 추가가
// 평범한 `yo`의 backend 시작 동작을 바꾸지 않는다.
#[test]
fn no_argument_keeps_the_live_defaults() {
    assert_eq!(
        parse([]).unwrap(),
        Command::Live(LiveOptions {
            mode: PresentationMode::Inline,
            glyph_profile: GlyphProfile::Rich,
            selection: LiveSelection::New,
        })
    );
}

// 명시한 UUID 재개와 현재 작업공간의 최근 세션 재개는 새 Session 시작과 구분되고,
// 동시에 지정하면 어느 쪽도 임의로 우선하지 않는다.
#[test]
fn live_continuation_options_are_explicit_and_mutually_exclusive() {
    let id = "01890f00-0000-7000-8000-000000000001";
    let Command::Live(resume) = parse(["--resume".into(), id.into()]).unwrap() else {
        panic!("--resume remains a live startup option");
    };
    assert_eq!(resume.selection, LiveSelection::Resume(id.parse().unwrap()));

    let Command::Live(continuation) = parse(["--continue".into()]).unwrap() else {
        panic!("--continue remains a live startup option");
    };
    assert_eq!(continuation.selection, LiveSelection::Continue);

    let error = parse(["--continue".into(), "--resume".into(), id.into()]).unwrap_err();
    assert!(error.to_string().contains("--resume"));
}

// 목록 option은 Session ID 없이 조합할 수 있고 `--details`가 선택 집합을 바꾸는 별도
// command가 아니라 같은 목록의 metadata 확장으로 해석된다.
#[test]
fn session_list_accepts_all_and_details_in_any_order() {
    let command = parse(["session".into(), "--details".into(), "--all".into()]).unwrap();

    assert_eq!(
        command,
        Command::Session(SessionCommand {
            session_id: None,
            all: true,
            details: true,
            view: SessionView::Chat,
            glyph_profile: GlyphProfile::Rich,
        })
    );
}

// full UUID 뒤의 Transcript view와 ASCII 선택은 저장 history를 읽는 표시 옵션으로만
// 결합되고 live presentation mode나 writer 설정으로 새지 않는다.
#[test]
fn direct_session_selects_a_read_only_projection() {
    let id = "01890f00-0000-7000-8000-000000000001";
    let command = parse([
        "session".into(),
        id.into(),
        "--view".into(),
        "transcript".into(),
        "--ascii".into(),
    ])
    .unwrap();

    let Command::Session(command) = command else {
        panic!("the session command remains distinct from live startup");
    };
    assert_eq!(command.session_id.unwrap().to_string(), id);
    assert_eq!(command.view, SessionView::Transcript);
    assert_eq!(command.glyph_profile, GlyphProfile::Ascii);
}

// list 전용 `--all`과 direct read UUID를 함께 쓰면 어느 쪽 의미도 임의로 우선하지 않고
// 사용법 오류로 거부해 조회 범위와 출력 대상이 모호해지지 않는다.
#[test]
fn list_only_options_are_rejected_for_a_direct_session() {
    let error = parse([
        "session".into(),
        "01890f00-0000-7000-8000-000000000001".into(),
        "--all".into(),
    ])
    .unwrap_err();

    assert!(error.to_string().contains("apply only to Session lists"));
}
