use std::fs;

use super::{
    repository::{GitRepository, success_json},
    support::{active_repository, candidate_request, direct_request, resolve},
};

// 사용자가 지식을 직접 지정하면 그 지식만 넣는 것이 아니라 실행에 필요한 모든 의존 지식도 함께
// 모은다. 의존 대상을 먼저 배치한 context를 만들고, 같은 요청은 새 디렉터리 대신 기존 build를
// 재사용한다.
#[test]
fn direct_root_includes_complete_closure_in_dependency_order_and_reuses() {
    let repository = active_repository();
    let request = direct_request(&repository, "knowledge_id", "tui.context.large", 8_000);

    let created = resolve(&repository, &request);
    let context_path = repository
        .path
        .join(created["context"]["path"].as_str().unwrap());
    let context = fs::read_to_string(context_path).unwrap();

    assert_eq!(created["status"], "created");
    assert_eq!(
        created["affected_ids"],
        serde_json::json!(["tui.context.base", "tui.context.large"])
    );
    assert!(context.find("tui.context.base").unwrap() < context.find("tui.context.large").unwrap());
    assert!(created["context"]["tokens"].as_u64().unwrap() > 0);

    let reused = resolve(&repository, &request);
    assert_eq!(reused["status"], "reused");
    assert_eq!(reused["build_id"], created["build_id"]);
    assert_eq!(reused["manifest"]["hash"], created["manifest"]["hash"]);
}

// 선택된 지식과 무관한 커밋이 trusted ref에 추가돼도 context 내용은 달라지지 않는다.
// 같은 content build를 재사용하되 resolve 응답의 trusted commit 관찰값은 최신으로 갱신한다.
#[test]
fn unrelated_trusted_commit_reuses_the_same_content_build() {
    let repository = active_repository();
    let request = direct_request(&repository, "symbol", "yo::base", 8_000);
    let initial = resolve(&repository, &request);

    fs::write(repository.path.join("unrelated.txt"), "unrelated\n").unwrap();
    repository.git(&["add", "unrelated.txt"]);
    repository.git(&["commit", "-m", "unrelated trusted change"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);

    let reused = resolve(&repository, &request);
    assert_eq!(reused["status"], "reused");
    assert_eq!(reused["build_id"], initial["build_id"]);
    assert_ne!(reused["trusted_commit"], initial["trusted_commit"]);
}

// 지식 본문 revision이 같아도 그것을 승인한 review evidence가 교체되면 근거 계보는 달라진다.
// context 본문 hash는 유지하되 build identity는 새 근거를 반영해 달라져야 한다.
#[test]
fn replacement_review_evidence_changes_build_identity() {
    let repository = active_repository();
    let request = direct_request(&repository, "knowledge_id", "tui.context.base", 8_000);
    let initial = resolve(&repository, &request);
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(
            repository
                .path
                .join(initial["manifest"]["path"].as_str().unwrap()),
        )
        .unwrap(),
    )
    .unwrap();
    let revision = manifest["plan"]["units"][0]["revision"].as_str().unwrap();
    let old_projection = manifest["plan"]["units"][0]["approval"]["projection_hash"]
        .as_str()
        .unwrap();
    let projection_request = repository.request(
        "replace-projection.json",
        &serde_json::json!({
            "schema": "methexis.review-projection-request/v1alpha1",
            "knowledge_id": "tui.context.base",
            "expected_revision": revision,
            "korean_markdown": "같은 지식 revision에 대한 새 검토 투영입니다.",
            "replace_projection_hash": old_projection
        }),
    );
    let projection =
        success_json(repository.run(&["project-review", projection_request.to_str().unwrap()]));
    let approval_request = repository.request(
        "replace-approval.json",
        &serde_json::json!({
            "schema": "methexis.approval-request/v1alpha1",
            "knowledge_id": "tui.context.base",
            "expected_revision": revision,
            "projection_hash": projection["hash"],
            "reviewer": "tui-architecture",
            "reviewed_at": "2026-07-25T12:00:00Z",
            "replace_revision": revision
        }),
    );
    success_json(repository.run(&["approve", approval_request.to_str().unwrap()]));
    repository.git(&["add", "methexis"]);
    repository.git(&["commit", "-m", "replace review evidence"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);

    let changed = resolve(&repository, &request);

    assert_ne!(changed["build_id"], initial["build_id"]);
    assert_eq!(changed["context"]["hash"], initial["context"]["hash"]);
}

// stale 상태라 context에서 제외된 candidate도 “왜 제외했는지”를 설명하는 build 근거의 일부다.
// 그 candidate의 review evidence가 바뀌면 context 본문은 같아도 build identity는 달라진다.
#[test]
fn omitted_stale_candidate_review_evidence_changes_build_identity() {
    let repository = GitRepository::code_approved();
    repository.integrate_active_checkpoint();
    fs::write(
        repository.path.join("methexis/code-source.txt"),
        "changed after activation\n",
    )
    .unwrap();
    let request = candidate_request(&repository, &[("tui.relocated", 100)], 8_000, false);
    let initial = resolve(&repository, &request);
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(
            repository
                .path
                .join(initial["manifest"]["path"].as_str().unwrap()),
        )
        .unwrap(),
    )
    .unwrap();
    let revision = repository.revision_for("tui.relocated");
    let old_projection = manifest["plan"]["observations"][0]["approval"]["projection_hash"]
        .as_str()
        .unwrap();
    let projection_request = repository.request(
        "replace-omitted-projection.json",
        &serde_json::json!({
            "schema": "methexis.review-projection-request/v1alpha1",
            "knowledge_id": "tui.relocated",
            "expected_revision": revision,
            "korean_markdown": "제외된 후보에 대한 새 검토 투영입니다.",
            "replace_projection_hash": old_projection
        }),
    );
    let projection =
        success_json(repository.run(&["project-review", projection_request.to_str().unwrap()]));
    let approval_request = repository.request(
        "replace-omitted-approval.json",
        &serde_json::json!({
            "schema": "methexis.approval-request/v1alpha1",
            "knowledge_id": "tui.relocated",
            "expected_revision": revision,
            "projection_hash": projection["hash"],
            "reviewer": "tui-architecture",
            "reviewed_at": "2026-07-25T12:00:00Z",
            "replace_revision": revision
        }),
    );
    success_json(repository.run(&["approve", approval_request.to_str().unwrap()]));
    repository.git(&["add", "methexis"]);
    repository.git(&["commit", "-m", "replace omitted review evidence"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);

    let changed = resolve(&repository, &request);

    assert_ne!(changed["build_id"], initial["build_id"]);
    assert_eq!(changed["context"]["hash"], initial["context"]["hash"]);
}

// 점수가 높은 candidate 묶음이 남은 token 예산보다 크다고 해서 선택을 바로 끝내지 않는다.
// 큰 묶음과 비활성 묶음은 이유를 기록해 건너뛰고, 뒤의 작은 묶음은 계속 context에 채운다.
#[test]
fn greedy_packing_skips_large_bundle_and_continues_to_small_candidate() {
    let repository = active_repository();
    let request = candidate_request(
        &repository,
        &[
            ("tui.context.large", 300),
            ("tui.context.small", 200),
            ("tui.context.inactive", 100),
        ],
        100,
        false,
    );

    let result = resolve(&repository, &request);
    let context = fs::read_to_string(
        repository
            .path
            .join(result["context"]["path"].as_str().unwrap()),
    )
    .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(
            repository
                .path
                .join(result["manifest"]["path"].as_str().unwrap()),
        )
        .unwrap(),
    )
    .unwrap();

    assert!(context.contains("tui.context.small"));
    assert!(!context.contains("tui.context.large"));
    assert!(!context.contains("tui.context.base"));
    let decisions = manifest["plan"]["candidate_decisions"].as_array().unwrap();
    assert_eq!(decisions[0]["reason"], "budget_exceeded");
    assert_eq!(decisions[1]["reason"], "fits_budget");
    assert_eq!(decisions[2]["reason"], "bundle_inactive");
}

// 같은 candidate 의미를 예쁜 JSON과 압축 JSON으로 전달하면 선택되는 지식은 같을 수 있다.
// 그래도 실제 입력 바이트가 달랐다는 사실을 보존하도록 build identity는 서로 달라야 한다.
#[test]
fn exact_candidate_bytes_change_build_identity_without_changing_selection() {
    let repository = active_repository();
    let compact = candidate_request(&repository, &[("tui.context.small", 100)], 8_000, false);
    let pretty = candidate_request(&repository, &[("tui.context.small", 100)], 8_000, true);

    let compact = resolve(&repository, &compact);
    let pretty = resolve(&repository, &pretty);

    assert_ne!(compact["build_id"], pretty["build_id"]);
    assert_eq!(compact["context"]["hash"], pretty["context"]["hash"]);
}

// 공백 표기나 anchor 순서만 다른 요청은 의미가 같으므로 서로 다른 build를 만들 필요가 없다.
// 요청을 정규화한 뒤 동일한 build identity를 공유하는지 확인한다.
#[test]
fn equivalent_direct_anchor_spelling_and_order_share_build_identity() {
    let repository = active_repository();
    let first = repository.request(
        "anchors-first.json",
        &serde_json::json!({
            "schema": "methexis.context-request/v1alpha1",
            "anchors": [
                {"kind": "symbol", "value": " yo::base "},
                {"kind": "knowledge_id", "value": "tui.context.small"}
            ],
            "tokenizer_profile": "o200k_base/v1",
            "max_tokens": 8000
        }),
    );
    let second = repository.request(
        "anchors-second.json",
        &serde_json::json!({
            "schema": "methexis.context-request/v1alpha1",
            "anchors": [
                {"kind": "knowledge_id", "value": "tui.context.small"},
                {"kind": "symbol", "value": "yo::base"}
            ],
            "tokenizer_profile": "o200k_base/v1",
            "max_tokens": 8000
        }),
    );

    let first = resolve(&repository, &first);
    let second = resolve(&repository, &second);

    assert_eq!(first["build_id"], second["build_id"]);
}
