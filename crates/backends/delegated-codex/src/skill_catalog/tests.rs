use std::{
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use yo_core::{
    ResolvedSkill, SkillAvailability, SkillReferenceCandidate, SkillReferenceScope,
    SkillReferenceSearchRequest, SubmissionRejectionKind,
};

use super::{
    SkillInterface, SkillMetadata, WireScope, candidate_from_wire, newest_request, skill_digest,
};

// Codex wire scope `repo`는 경로 추측 없이 Workspace 출처로 매핑되고,
// interface의 짧은 이름과 설명이 일반 메타데이터보다 우선한다.
#[test]
fn wire_metadata_maps_to_honest_workspace_provenance() {
    let candidate = candidate_from_wire(
        "local-host:fixture",
        SkillMetadata {
            name: "raw-name".to_owned(),
            description: "Long description".to_owned(),
            short_description: Some("Legacy description".to_owned()),
            path: "/workspace/.agents/skills/review/SKILL.md".to_owned(),
            scope: WireScope::Repo,
            enabled: true,
            interface: Some(SkillInterface {
                display_name: Some("Review".to_owned()),
                short_description: Some("Review changes".to_owned()),
            }),
        },
        3,
        Ok("sha256:exact".to_owned()),
    );

    assert_eq!(
        candidate.reference().scope(),
        SkillReferenceScope::Workspace
    );
    assert_eq!(candidate.display_name(), "Review");
    assert_eq!(candidate.description(), "Review changes");
    assert_eq!(
        candidate.reference().execution_environment_identity(),
        "local-host:fixture"
    );
    assert_eq!(candidate.reference().catalog_generation(), 3);
    assert_eq!(candidate.availability(), &SkillAvailability::Enabled);
}

// 비활성 Codex 항목은 목록에는 남지만 선택 불가 이유를 함께 보존한다.
#[test]
fn disabled_wire_skill_remains_visible_with_a_reason() {
    let candidate = candidate_from_wire(
        "local-host:fixture",
        SkillMetadata {
            name: "review".to_owned(),
            description: "Review changes".to_owned(),
            short_description: None,
            path: "/skills/review/SKILL.md".to_owned(),
            scope: WireScope::User,
            enabled: false,
            interface: None,
        },
        1,
        Ok("sha256:exact".to_owned()),
    );

    assert!(
        matches!(candidate.availability(), SkillAvailability::Disabled(reason) if reason == "Disabled by Codex configuration")
    );
}

// Codex가 enabled로 보고해도 exact revision을 읽을 수 없으면 선택을 허용하지 않아,
// 제출 시점 재검증이 비교할 수 없는 reference가 UI에서 만들어지지 않는다.
#[test]
fn missing_revision_disables_an_otherwise_enabled_skill() {
    let candidate = candidate_from_wire(
        "local-host:test",
        SkillMetadata {
            name: "review".to_owned(),
            description: "Review changes".to_owned(),
            short_description: None,
            path: "/missing/review/SKILL.md".to_owned(),
            scope: WireScope::User,
            enabled: true,
            interface: None,
        },
        1,
        Err("Skill revision unavailable: missing".to_owned()),
    );

    assert!(
        matches!(candidate.availability(), SkillAvailability::Disabled(reason) if reason == "Skill revision unavailable: missing")
    );
    assert_eq!(candidate.reference().entry_revision(), "unavailable");
}

// 새 overlay의 refresh 요청 뒤 연속 입력 요청이 queue에서 합쳐져도 최신 query를 쓰면서
// refresh 의도는 보존해, 오래된 catalog가 우연히 재사용되지 않는다.
#[test]
fn request_coalescing_preserves_catalog_refresh_intent() {
    let (sender, receiver) = mpsc::channel();
    sender
        .send(SkillReferenceSearchRequest::new(
            2,
            2,
            3,
            0..3,
            "$re",
            "re",
            false,
        ))
        .unwrap();
    let first = SkillReferenceSearchRequest::new(1, 1, 1, 0..1, "$", "", true);

    let (latest, refresh) = newest_request(first, &receiver);

    assert_eq!(latest.request_id(), 2);
    assert_eq!(latest.query(), "re");
    assert!(refresh);
}

struct SkillFixture(PathBuf);

impl SkillFixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("yo-skill-catalog-{}", uuid::Uuid::now_v7()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for SkillFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn metadata_at(path: &Path) -> SkillMetadata {
    let path = path.to_str().unwrap();
    SkillMetadata {
        name: "fixture".to_owned(),
        description: "Fixture skill".to_owned(),
        short_description: None,
        path: path.to_owned(),
        scope: WireScope::User,
        enabled: true,
        interface: None,
    }
}

fn candidate_at(path: &Path) -> SkillReferenceCandidate {
    candidate_from_wire(
        "local-host:test",
        metadata_at(path),
        1,
        skill_digest(path.to_str().unwrap()),
    )
}

fn wait_bounded(mut child: Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("catalog test process did not finish: {result:?}");
            },
        }
    }
}

// 정상 UTF-8 파일과 심볼릭 링크는 같은 기존 digest를 유지하고 정확한 최대 크기를 허용한다.
#[test]
fn regular_skill_snapshots_preserve_digest_symlinks_and_exact_limit() {
    let fixture = SkillFixture::new();
    let path = fixture.0.join("SKILL.md");
    let alias = fixture.0.join("alias.md");
    fs::write(&path, "hello").unwrap();
    symlink(&path, &alias).unwrap();
    for path in [&path, &alias] {
        let candidate = candidate_at(path);
        assert_eq!(candidate.availability(), &SkillAvailability::Enabled);
        assert_eq!(
            candidate.reference().entry_revision(),
            "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
    fs::write(&path, "x".repeat(ResolvedSkill::MAX_INSTRUCTION_BYTES)).unwrap();
    assert_eq!(
        candidate_at(&path).availability(),
        &SkillAvailability::Enabled
    );
    fs::write(&path, "x".repeat(ResolvedSkill::MAX_INSTRUCTION_BYTES + 1)).unwrap();
    let candidate = candidate_at(&path);
    assert!(
        matches!(candidate.availability(), SkillAvailability::Disabled(reason)
        if reason.contains("instruction byte limit exceeded"))
    );
    assert_eq!(candidate.reference().entry_revision(), "unavailable");
}

// 잘못된 UTF-8·읽을 수 없는 경로·일반 파일이 아닌 항목은 이유를 가진 비활성 행으로 남는다.
#[test]
fn unusable_skill_files_remain_disabled_catalog_rows() {
    let fixture = SkillFixture::new();
    let invalid = fixture.0.join("invalid.md");
    fs::write(&invalid, [0xff]).unwrap();
    for path in [&invalid, &fixture.0.join("missing.md"), &fixture.0] {
        let candidate = candidate_at(path);
        assert_eq!(candidate.display_name(), "fixture");
        assert_eq!(candidate.reference().entry_revision(), "unavailable");
        assert!(
            matches!(candidate.availability(), SkillAvailability::Disabled(reason)
            if reason.starts_with("Skill revision unavailable:"))
        );
    }
}

// 작성자가 없는 FIFO도 즉시 비활성화하며 회귀 시 자식 프로세스를 종료해 테스트가 멈추지 않는다.
#[test]
fn fifo_skill_is_rejected_without_waiting_for_a_writer() {
    const CHILD_PATH: &str = "YO_SKILL_CATALOG_FIFO_TEST_PATH";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        let candidate = candidate_at(Path::new(&path));
        assert!(
            matches!(candidate.availability(), SkillAvailability::Disabled(reason)
            if reason.contains("not a regular file"))
        );
        return;
    }
    let fixture = SkillFixture::new();
    let fifo = fixture.0.join("SKILL.md");
    assert!(wait_bounded(Command::new("mkfifo").arg(&fifo).spawn().unwrap()).success());
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "skill_catalog::tests::fifo_skill_is_rejected_without_waiting_for_a_writer",
        ])
        .env(CHILD_PATH, &fifo)
        .spawn()
        .unwrap();
    assert!(wait_bounded(child).success());
}

// 다른 카탈로그 generation 때문에 기존 selector를 바꾸지 않고, 검증한 원문을 snapshot으로
// 보존한다. 이후 경로의 파일이 바뀌어도 이미 준비한 model input은 재조회하지 않는다.
#[test]
fn admitted_skill_keeps_original_reference_and_frozen_file_bytes() {
    use super::{resolve_selected_skill, validate_selected_skill};
    let fixture = SkillFixture::new();
    let path = fixture.0.join("SKILL.md");
    let text = "# Fixture\nRead references/later.md only when needed.\n";
    fs::write(&path, text).unwrap();
    let selected = candidate_at(&path).reference().clone();
    validate_selected_skill("local-host:test", &selected, vec![metadata_at(&path)]).unwrap();
    let snapshot = resolve_selected_skill(selected.clone()).unwrap();
    fs::write(&path, "changed").unwrap();
    assert_eq!(snapshot.reference(), &selected);
    assert_eq!(snapshot.instructions(), text);
    assert_eq!(
        resolve_selected_skill(selected).unwrap_err().kind(),
        SubmissionRejectionKind::StaleReference
    );
}

// 삭제·비활성화·동일 locator의 중복 entry·이름 변경은 파일 로드 전에 descriptor 단계에서
// 거절한다. 변경된 body와 첫 용량 초과도 typed rejection으로 끝난다.
#[test]
fn selected_skill_admission_rejects_changed_or_ambiguous_authority() {
    use yo_core::SubmissionRejectionKind;

    use super::{resolve_selected_skill, validate_selected_skill};
    let fixture = SkillFixture::new();
    let path = fixture.0.join("SKILL.md");
    fs::write(&path, "original").unwrap();
    let selected = candidate_at(&path).reference().clone();
    assert_eq!(
        validate_selected_skill("local-host:test", &selected, vec![])
            .unwrap_err()
            .kind(),
        SubmissionRejectionKind::StaleReference
    );
    let mut disabled = metadata_at(&path);
    disabled.enabled = false;
    assert_eq!(
        validate_selected_skill("local-host:test", &selected, vec![disabled])
            .unwrap_err()
            .kind(),
        SubmissionRejectionKind::Unauthorized
    );
    assert_eq!(
        validate_selected_skill(
            "local-host:test",
            &selected,
            vec![metadata_at(&path), metadata_at(&path)]
        )
        .unwrap_err()
        .kind(),
        SubmissionRejectionKind::InvalidReference
    );
    let mut renamed = metadata_at(&path);
    renamed.name = "other".to_owned();
    assert_eq!(
        validate_selected_skill("local-host:test", &selected, vec![renamed])
            .unwrap_err()
            .kind(),
        SubmissionRejectionKind::StaleReference
    );
    fs::write(&path, "x".repeat(ResolvedSkill::MAX_INSTRUCTION_BYTES + 1)).unwrap();
    assert_eq!(
        resolve_selected_skill(selected.clone()).unwrap_err().kind(),
        SubmissionRejectionKind::OverBudget
    );
    fs::remove_file(&path).unwrap();
    assert_eq!(
        resolve_selected_skill(selected).unwrap_err().kind(),
        SubmissionRejectionKind::RequiredAssetUnavailable
    );
}

// 혼합 입력의 workspace 실패는 Codex catalog process 시작과 skill 파일 읽기보다 먼저
// 반환된다. 따라서 설치되지 않은 catalog executable까지 도달하지 않는다.
#[test]
fn invalid_workspace_precedes_catalog_access_for_mixed_input() {
    use yo_core::{
        InputAdmissionHost, InputReference, UserInput, WorkspaceHostId, WorkspaceReference,
        WorkspaceReferenceKind,
    };

    use super::CodexSkillInputAdmission;
    use crate::CodexBackendConfig;
    let fixture = SkillFixture::new();
    let path = fixture.0.join("SKILL.md");
    fs::write(&path, "original").unwrap();
    let reference = candidate_at(&path).reference().clone();
    let workspace = WorkspaceReference::new(
        "forged",
        "other-host",
        "other-workspace",
        "other-root",
        "file.rs",
        WorkspaceReferenceKind::File,
    )
    .unwrap();
    let input = UserInput::with_references(
        "use @file.rs $fixture",
        vec![
            InputReference::workspace(4..12, workspace),
            InputReference::skill(13..21, reference),
        ],
    )
    .unwrap();
    let host = CodexSkillInputAdmission::new(
        CodexBackendConfig::new(&fixture.0).with_executable(fixture.0.join("missing-codex")),
        WorkspaceHostId::new().unwrap(),
        None,
    )
    .unwrap();
    let error = host.prepare(&input).unwrap_err();
    assert!(
        error
            .message()
            .contains("another execution environment or workspace"),
        "{}",
        error.message()
    );
}

// 실제 stdio catalog 교환부터 prepare 결과까지 검증한다. forceReload 요청으로 선택
// eligibility를 갱신하고, 준비된 snapshot은 이후 파일 변경에 영향을 받지 않는다.
#[test]
fn catalog_process_admission_returns_verified_immutable_instructions() {
    use std::os::unix::fs::PermissionsExt;

    use yo_core::{InputAdmissionHost, InputReference, UserInput, WorkspaceHostId};

    use super::{CodexSkillInputAdmission, skill_revision};
    use crate::CodexBackendConfig;

    let fixture = SkillFixture::new();
    let path = fixture.0.join("SKILL.md");
    let instructions = "# Fixture\nKeep complete instructions.\n";
    fs::write(&path, instructions).unwrap();
    let host_id = WorkspaceHostId::new().unwrap();
    let environment = format!("local-host:{host_id}");
    let selected = candidate_from_wire(
        &environment,
        metadata_at(&path),
        7,
        Ok(skill_revision(instructions)),
    )
    .reference()
    .clone();
    let catalog = serde_json::json!({"data": [{
        "cwd": fixture.0, "errors": [], "skills": [{
            "name": "fixture", "description": "Fixture skill", "path": path,
            "scope": "user", "enabled": true
        }]
    }]});
    let executable = fixture.0.join("catalog.py");
    let script = format!(
        r##"#!/usr/bin/env python3
import json, sys
catalog = json.loads({catalog_literal})
for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    if request["method"] == "initialize":
        result = {{"userAgent": "codex/0.149.0", "platformFamily": "unix", "platformOs": "linux"}}
    elif request["method"] == "skills/list":
        assert request["params"]["forceReload"] is True
        assert request["params"]["cwds"] == [catalog["data"][0]["cwd"]]
        result = catalog
    else:
        raise AssertionError(request["method"])
    print(json.dumps({{"id": request["id"], "result": result}}), flush=True)
"##,
        catalog_literal = serde_json::to_string(&catalog.to_string()).unwrap()
    );
    fs::write(&executable, script).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let admission = CodexSkillInputAdmission::new(
        CodexBackendConfig::new(&fixture.0)
            .with_executable(executable)
            .with_request_timeout(Duration::from_secs(3)),
        host_id,
        None,
    )
    .unwrap();
    let input = UserInput::with_references(
        "$fixture",
        vec![InputReference::skill(0..8, selected.clone())],
    )
    .unwrap();
    let snapshot = admission.prepare(&input).unwrap().unwrap();
    assert_eq!(snapshot.reference(), &selected);
    assert_eq!(snapshot.instructions(), instructions);
    fs::write(&path, "changed").unwrap();
    assert_eq!(snapshot.instructions(), instructions);
    assert_eq!(
        admission.prepare(&input).unwrap_err().kind(),
        SubmissionRejectionKind::StaleReference
    );
}
