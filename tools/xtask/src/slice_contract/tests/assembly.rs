use std::path::PathBuf;

use super::support::{commit, contract_for_ref};
use crate::{slice_contract::check_wave_assembly, test_support::TestRepository};

// 같은 Wave base의 provider별 component 두 개가 서로와 공용 조립 경계를 침범하지
// 않으면 한 assembly owner 아래에서 병렬 dispatch할 수 있습니다.
#[test]
fn wave_assembly_accepts_disjoint_components_and_one_explicit_owner() {
    let fixture = Fixture::new("wave-assembly-pass");
    let kimi = fixture.component(
        "kimi-backend",
        "crates/connectors/kimi/**",
        "agent.connector.kimi",
    );
    let qwen = fixture.component(
        "qwen-backend",
        "crates/backends/managed/src/qwen/**",
        "agent.backend.qwen",
    );

    check_wave_assembly(&fixture.repository.path, &fixture.boundary, &[kimi, qwen]).unwrap();
}

// component가 assembly owner에게 미뤄 둔 Cargo.lock을 자신의 write-set에도 넣으면
// 공용 조립과 provider 의미 변경이 다시 섞이므로 dispatch 전에 거절합니다.
#[test]
fn wave_assembly_rejects_component_write_paths_reserved_for_assembly() {
    let fixture = Fixture::new("wave-assembly-path-overlap");
    let component = fixture.component("kimi-backend", "Cargo.lock", "agent.connector.kimi");

    let error =
        check_wave_assembly(&fixture.repository.path, &fixture.boundary, &[component]).unwrap_err();

    assert!(error.contains("claims write paths reserved for assembly owner"));
    assert!(error.contains("Cargo.lock <> Cargo.lock"));
}

// component와 assembly owner가 같은 semantic contract를 동시에 소유한다고 선언하면
// 파일 경로가 달라도 의사결정 소유권이 중복되므로 사전검사가 실패합니다.
#[test]
fn wave_assembly_rejects_contract_ownership_reserved_for_assembly() {
    let fixture = Fixture::new("wave-assembly-contract-overlap");
    let component = fixture.component(
        "backend-composition-child",
        "crates/connectors/kimi/**",
        "agent.backend.composition",
    );

    let error =
        check_wave_assembly(&fixture.repository.path, &fixture.boundary, &[component]).unwrap_err();

    assert!(error.contains("claims contracts reserved for assembly owner"));
    assert!(error.contains("agent.backend.composition"));
}

// assembly 경계가 분리되어 있어도 component끼리 같은 subtree를 임대하면 실제 병렬
// 작업은 안전하지 않으므로 기존 Slice 병렬 계약을 함께 적용합니다.
#[test]
fn wave_assembly_rejects_overlap_between_components() {
    let fixture = Fixture::new("wave-assembly-component-overlap");
    let broad = fixture.component(
        "managed-backend",
        "crates/backends/managed/**",
        "agent.backend.managed",
    );
    let narrow = fixture.component(
        "qwen-backend",
        "crates/backends/managed/src/qwen/**",
        "agent.backend.qwen",
    );

    let error = check_wave_assembly(
        &fixture.repository.path,
        &fixture.boundary,
        &[broad, narrow],
    )
    .unwrap_err();

    assert!(error.contains("overlapping write leases"));
}

// component 계약과 boundary를 고정한 뒤 Wave ref가 전진하면 이전 분할은 더 이상 현재
// dispatch 기준이 아니므로 새 base에서 다시 계획하도록 stale로 거절합니다.
#[test]
fn wave_assembly_rejects_a_stale_wave_base() {
    let fixture = Fixture::new("wave-assembly-stale");
    let component = fixture.component(
        "kimi-backend",
        "crates/connectors/kimi/**",
        "agent.connector.kimi",
    );
    fixture.repository.write("accepted.txt", "advanced Wave\n");
    fixture.repository.git(["add", "accepted.txt"]);
    fixture
        .repository
        .git(["commit", "--quiet", "-m", "advance Wave"]);

    let error =
        check_wave_assembly(&fixture.repository.path, &fixture.boundary, &[component]).unwrap_err();

    assert!(error.contains("Wave assembly base"));
    assert!(error.contains("is stale"));
}

// `direct`는 Wave 위치에서만 예약되며 Slice leaf로는 유효합니다. assembly owner와
// component 이름에 이를 허용해 기존 branch grammar보다 좁은 새 계약을 만들지 않습니다.
#[test]
fn wave_assembly_allows_direct_as_a_slice_leaf() {
    let fixture = Fixture::new_with_assembly("wave-assembly-direct-slice", "direct");
    let component = fixture.component(
        "provider-backend",
        "crates/connectors/provider/**",
        "agent.connector.provider",
    );

    check_wave_assembly(&fixture.repository.path, &fixture.boundary, &[component]).unwrap();
}

// 공백 외의 Git 금지 문자를 통과시키면 이후 실제 Slice branch 생성에서야 실패하므로,
// boundary preflight가 Git의 ref 문법과 같은 지점에서 잘못된 owner 이름을 거절합니다.
#[test]
fn wave_assembly_rejects_an_invalid_git_ref_segment() {
    let fixture = Fixture::new_with_assembly("wave-assembly-invalid-segment", "bad:name");
    let component = fixture.component(
        "provider-backend",
        "crates/connectors/provider/**",
        "agent.connector.provider",
    );

    let error =
        check_wave_assembly(&fixture.repository.path, &fixture.boundary, &[component]).unwrap_err();

    assert!(error.contains("assembly Slice name must be one non-reserved branch segment"));
}

struct Fixture {
    repository: TestRepository,
    base: String,
    base_ref: &'static str,
    boundary: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        Self::new_with_assembly(name, "backend-composition")
    }

    fn new_with_assembly(name: &str, assembly_slice: &str) -> Self {
        let repository = TestRepository::new(name);
        repository.write("README.md", "base\n");
        let base = commit(&repository);
        repository.git(["switch", "--quiet", "-c", "wave/backend-work"]);
        let base_ref = "refs/heads/wave/backend-work";
        let boundary = repository.write(
            "assembly.json",
            &format!(
                r#"{{
  "schema": "yo.wave-assembly-boundary/v1alpha1",
  "wave": "backend-work",
  "base": "{base}",
  "base_ref": "{base_ref}",
  "assembly_slice": "{assembly_slice}",
  "owned_contracts": ["agent.backend.composition"],
  "allowed_write_set": ["Cargo.lock", "crates/yo-cli/src/backend.rs", "README.md"]
}}"#
            ),
        );
        Self {
            repository,
            base,
            base_ref,
            boundary,
        }
    }

    fn component(&self, slice: &str, path: &str, owned_contract: &str) -> PathBuf {
        self.repository.write(
            format!("{slice}.json"),
            &contract_for_ref(slice, &self.base, self.base_ref, path, owned_contract),
        )
    }
}
