use super::*;

// 단순 제품 실패는 clap과 같은 짧은 `error:` 선두 형식으로 보이고, 파이프나
// NO_COLOR 경로가 사용할 plain rendering에는 ANSI 제어 문자가 섞이지 않습니다.
#[test]
fn plain_message_is_compact_and_color_free() {
    let error = AppError::message("stored Session was not found");

    assert_eq!(error.render(false), "error: stored Session was not found\n");
    assert!(!error.render(false).contains("\x1b["));
}

// 작업 context가 있는 실패는 context를 결과 문장으로, 원래 오류를 별도 원인으로
// 보존해 긴 중첩 문자열을 한 줄로 합치지 않고도 두 정보를 모두 전달합니다.
#[test]
fn contextual_failure_separates_operation_from_cause() {
    let error = AppError::single(
        "reading Yo configuration",
        "unknown field `api_protocol` at line 4",
    );

    assert_eq!(
        error.render(false),
        "error: reading Yo configuration failed\n\nCaused by:\n  unknown field `api_protocol` at line 4\n"
    );
    assert_eq!(
        error.to_string(),
        "reading Yo configuration: unknown field `api_protocol` at line 4"
    );
}

// startup target 부재처럼 사용자가 바로 복구할 수 있는 실패는 설명에 명령을 섞지 않고
// 복사 가능한 각 명령을 독립된 tip 행으로 제공하며 내부 help 구조도 그대로 유지합니다.
#[test]
fn recovery_commands_are_structured_and_copyable() {
    let error = AppError::message("no startup target is selected")
        .with_help(["yo connect", "yo --model host:codex"]);

    assert_eq!(
        error.render(false),
        "error: no startup target is selected\n\ntip: try one of these commands\n\n  yo connect\n  yo --model host:codex\n"
    );
    assert_eq!(error.help(), ["yo connect", "yo --model host:codex"]);
}

// cleanup처럼 여러 독립 실패가 함께 발생하면 첫 실패만 남기거나 세미콜론 한 줄로
// 뭉치지 않고, 안정된 입력 순서의 목록으로 모든 실패를 한 번씩 보여 줍니다.
#[test]
fn multiple_failures_render_as_an_ordered_list() {
    let error = AppError::many([
        "terminal session: output failed".to_owned(),
        "agent cleanup: worker stopped".to_owned(),
    ]);

    assert_eq!(
        error.render(false),
        "error: multiple operations failed\n\n  - terminal session: output failed\n  - agent cleanup: worker stopped\n"
    );
}

// 실행 실패와 cleanup 실패를 상위 경계에서 합쳐도 원래 오류가 제공한 복구 명령은
// 사라지지 않으며, 같은 명령이 여러 오류에 있으면 사용자에게 한 번만 제시됩니다.
#[test]
fn combining_failures_preserves_and_deduplicates_recovery_commands() {
    let combined = AppError::combine([
        AppError::message("no startup target is selected")
            .with_help(["yo connect", "yo --model host:codex"]),
        AppError::message("cleanup failed").with_help(["yo connect"]),
    ]);

    assert_eq!(combined.help(), ["yo connect", "yo --model host:codex"]);
    assert!(combined.render(false).contains("  - cleanup failed\n"));
}

// context와 원인을 가진 오류 여러 개를 합쳐도 각각의 의미가 안정된 목록 항목으로
// 남고, 함께 제공된 중복 복구 명령은 목록 뒤에 한 번만 출력됩니다.
#[test]
fn combining_contextual_failures_preserves_causes_and_help() {
    let combined = AppError::combine([
        AppError::single("reading configuration", "unknown field").with_help(["yo connect"]),
        AppError::single("opening credentials", "permission denied").with_help(["yo connect"]),
    ]);

    assert_eq!(
        combined.render(false),
        "error: multiple operations failed\n\n  - reading configuration: unknown field\n  - opening credentials: permission denied\n\ntip: try one of these commands\n\n  yo connect\n"
    );
}

// interactive stderr용 style은 상태 label과 tip label에만 적용되고, 메시지와 복구 명령
// 자체는 기본 전경색을 유지해 밝고 어두운 terminal theme 모두를 존중합니다.
#[test]
fn styled_rendering_colors_only_semantic_labels() {
    let rendered = AppError::message("no startup target is selected")
        .with_help(["yo connect"])
        .render(true);

    assert!(rendered.starts_with("\x1b[1;31merror:\x1b[0m "));
    assert!(rendered.contains("\x1b[1;36mtip:\x1b[0m"));
    assert!(!rendered.contains("\x1b[1;31mno startup"));
    assert!(!rendered.contains("\x1b[1;36myo connect"));
}
