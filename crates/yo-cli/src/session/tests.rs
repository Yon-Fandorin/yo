use super::*;

fn unbounded(rows: &[SessionRow], all: bool, details: bool) -> String {
    format_rows(
        rows,
        all,
        details,
        OutputWidth::Unbounded,
        HeadingStyle::Plain,
    )
    .unwrap()
}

fn row(resume: &str, workspace: &str) -> SessionRow {
    SessionRow {
        resume: resume.to_owned(),
        status: "available".to_owned(),
        workspace: workspace.to_owned(),
        updated: "1700000002000".to_owned(),
        started: "1700000001000".to_owned(),
        version: "v1".to_owned(),
        continuation: "unavailable".to_owned(),
        path: format!("/work/{workspace}"),
        detail: String::new(),
    }
}

// 기본 목록은 사용자가 바로 고를 UUID와 상태/시간만 보여주고, 현재 workspace와 중복되는
// WORKSPACE 및 schema 세부사항은 넣지 않아 짧은 terminal에서도 핵심 열이 유지된다.
#[test]
fn ordinary_list_keeps_the_compact_column_order() {
    let output = unbounded(&[row("session-a", "yo")], false, false);
    let header = output.lines().next().unwrap();

    assert_eq!(
        header.split_whitespace().collect::<Vec<_>>(),
        ["RESUME", "STATUS", "UPDATED", "STARTED"]
    );
    assert!(!output.contains("WORKSPACE"));
    assert!(!output.contains("VERSION"));
}

// `--all --details`는 다른 workspace를 구분할 짧은 WORKSPACE를 날짜보다 앞에 두고,
// 검수용 schema/continuation/full path/reason을 뒤에 확장하되 UUID 열은 그대로 유지한다.
#[test]
fn all_details_expands_metadata_without_replacing_the_resume_identity() {
    let output = unbounded(&[row("session-a", "yo")], true, true);
    let header = output.lines().next().unwrap();

    assert_eq!(
        header.split_whitespace().collect::<Vec<_>>(),
        [
            "RESUME",
            "STATUS",
            "WORKSPACE",
            "UPDATED",
            "STARTED",
            "VERSION",
            "CONTINUATION",
            "PATH",
            "DETAIL",
        ]
    );
    assert!(output.contains("/work/yo"));
}

// Session이 하나도 없는 새 머신은 설명 문장을 stdout 데이터처럼 출력하지 않고 빈 성공
// 결과를 반환해 `yo session | ...` 파이프가 실제 row만 다룰 수 있게 한다.
#[test]
fn empty_list_has_empty_stdout() {
    assert_eq!(unbounded(&[], false, false), "");
}

// stdout이 terminal이면 측정한 폭을 사용하고, 측정 실패 시에도 80셀로 복구하지만,
// 파이프 출력은 폭과 무관한 한 줄 형식을 유지해 shell 조합의 결과가 안정적입니다.
#[test]
fn output_width_policy_distinguishes_terminals_from_pipes() {
    let observed = NonZeroU16::new(120).unwrap();

    assert_eq!(
        output_width(true, Ok(observed)),
        OutputWidth::Bounded(observed)
    );
    assert_eq!(
        output_width(true, Err(std::io::Error::other("unavailable"))),
        OutputWidth::Bounded(NonZeroU16::new(80).unwrap())
    );
    assert_eq!(output_width(false, Ok(observed)), OutputWidth::Unbounded);
    assert_eq!(heading_style(true), HeadingStyle::BoldAnsi);
    assert_eq!(heading_style(false), HeadingStyle::Plain);
}

// 상세 목록이 terminal 폭을 넘으면 PATH와 DETAIL을 함께 표 아래로 옮기되, 각
// label/value pair가 전체 폭에 들어가면 독립된 한 줄에서 불필요한 개행 없이 읽습니다.
#[test]
fn narrow_details_fold_path_and_detail_below_the_primary_row() {
    let mut value = row("session-a", "yo");
    value.detail = "reason".to_owned();

    let output = format_rows(
        &[value],
        true,
        true,
        OutputWidth::Bounded(NonZeroU16::new(80).unwrap()),
        HeadingStyle::Plain,
    )
    .unwrap();

    assert!(output.contains("PATH  /work/yo\n"));
    assert!(output.contains("DETAIL  reason\n"));
}

// 저장소 오류의 제어문자는 표 밖으로 cursor를 움직이지 않고 읽을 수 있는 escape로
// 바뀌어, 상세 목록의 한 row가 다른 row나 terminal 상태를 손상하지 않습니다.
#[test]
fn table_diagnostics_escape_control_characters() {
    assert_eq!(terminal_safe("bad\npath\u{1b}"), "bad\\npath\\u{1b}");
}

// 기본 Chat stdout은 pipe 가능한 본문만 유지하되 v1이 volatile suffix 부재를 증명하지
// 못한다는 경계는 stderr 진단으로 노출하고, 같은 사실을 본문에 적는 Transcript는 중복하지 않습니다.
#[test]
fn chat_warns_when_durability_continuity_is_not_observable() {
    let session_id = "01890f00-0000-7000-8000-000000000001".parse().unwrap();

    let chat = archival_diagnostics(
        session_id,
        SessionView::Chat,
        StoredSessionContinuity::NotObservable,
        true,
    );
    let transcript = archival_diagnostics(
        session_id,
        SessionView::Transcript,
        StoredSessionContinuity::NotObservable,
        true,
    );

    assert_eq!(chat.len(), 1);
    assert!(chat[0].contains("volatile suffix"));
    assert!(transcript.is_empty());
}
