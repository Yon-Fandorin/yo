use super::*;

// `discover`가 계약 순서인 최신 UPDATED 우선으로 건넨 목록에서 `--continue`는
// 다른 Host·workspace와 unavailable 항목을 건너뛰고 첫 eligible Session을 고른다.
#[test]
fn continue_selection_keeps_discovery_order_and_filters_execution_identity() {
    let host = WorkspaceHostId::new().unwrap();
    let other_host = WorkspaceHostId::new().unwrap();
    let cwd = std::env::current_dir().unwrap();
    let workspace = HostWorkspacePath::normalize_local(&cwd).unwrap();
    let other_workspace = HostWorkspacePath::normalize_local(cwd.parent().unwrap()).unwrap();
    let matching = candidate(4, host, workspace.clone(), true);
    let sessions = [
        candidate(5, other_host, workspace.clone(), true),
        candidate(3, host, other_workspace, true),
        candidate(2, host, workspace.clone(), false),
        matching.clone(),
    ];

    assert_eq!(
        select_continue_from(sessions, host, &workspace),
        Some(matching.session_id)
    );
}

// 현재 Host와 workspace에 실행 가능한 후보가 없으면 `--continue`는 임의의 다른
// Session을 선택하지 않으며, 호출자가 새 Session을 만들지 않는 실패로 처리할 수 있다.
#[test]
fn continue_selection_returns_none_without_an_eligible_workspace_candidate() {
    let host = WorkspaceHostId::new().unwrap();
    let workspace = HostWorkspacePath::normalize_local(std::env::current_dir().unwrap()).unwrap();
    let sessions = [candidate(
        1,
        WorkspaceHostId::new().unwrap(),
        workspace.clone(),
        true,
    )];

    assert_eq!(select_continue_from(sessions, host, &workspace), None);
}

// writer lease 획득부터 native backend 재개까지 어느 단계에서 실패해도 명시적
// `--resume`은 입력 가능한 Session을 만들지 않고 같은 Session의 읽기 전용 표시로 닫힌다.
#[test]
fn resume_launch_failures_are_classified_as_read_only_before_input_admission() {
    let session_id: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let stages = [
        ResumeFailureStage::WritableStorage,
        ResumeFailureStage::Revalidation,
        ResumeFailureStage::RecordedWorkspace,
        ResumeFailureStage::WorkspaceReferences,
        ResumeFailureStage::SkillReferences,
        ResumeFailureStage::BackendSpawn,
        ResumeFailureStage::NativeResume,
    ];

    for stage in stages {
        let disposition =
            classify_launch_failure(LiveSelection::Resume(session_id), stage, "fixture failure");
        assert!(matches!(
            disposition,
            ResumeFailureDisposition::ReadOnly {
                session_id: actual,
                reason,
            } if actual == session_id && reason.contains("fixture failure")
        ));
    }
}

// 새 Session 시작 실패는 과거 기록을 보여 줄 대상이 없으므로 읽기 전용으로 위장하지
// 않고 오류로 남기며, 내부 전용 `Continue` 상태도 같은 fail-closed 분류를 따른다.
#[test]
fn non_resume_launch_failures_remain_errors() {
    for selection in [LiveSelection::New, LiveSelection::Continue] {
        assert!(matches!(
            classify_launch_failure(
                selection,
                ResumeFailureStage::BackendSpawn,
                "fixture failure"
            ),
            ResumeFailureDisposition::Abort(reason) if reason.contains("fixture failure")
        ));
    }
}

// `--continue`가 preflight에서 구체 Session을 찾았더라도 이후 실패 정책에는 원래
// 요청이 보존되어, 내부 `Resume(id)` 변환 때문에 읽기 전용 fallback으로 바뀌지 않는다.
#[test]
fn resolved_continue_preserves_its_abort_policy() {
    let session_id: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let preparation = LivePreparation::Resume {
        session_id,
        failure_selection: LiveSelection::Continue,
    };
    let LivePreparation::Resume {
        failure_selection, ..
    } = preparation
    else {
        panic!("the fixture is a resolved continuation")
    };

    assert!(matches!(
        classify_launch_failure(
            failure_selection,
            ResumeFailureStage::NativeResume,
            "fixture failure"
        ),
        ResumeFailureDisposition::Abort(_)
    ));
}

fn candidate(
    id: u8,
    host: WorkspaceHostId,
    workspace: HostWorkspacePath,
    eligible: bool,
) -> ContinueCandidate {
    ContinueCandidate {
        session_id: format!("01890f00-0000-7000-8000-{id:012}").parse().unwrap(),
        eligible,
        host,
        workspace,
    }
}
