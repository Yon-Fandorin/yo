use std::{collections::BTreeMap, fs, path::PathBuf};

use super::{
    load,
    records::{SOURCE_SCHEMA, SourcePayload, SourceRecord},
};
use crate::{
    discovery,
    test_support::TestDirectory,
    wire::{DiscoveryRequest, REQUEST_SCHEMA},
};

const HASH: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

fn write_knowledge(root: &TestDirectory, relative: &str, id: &str) {
    let owners = root.path().join("methexis/owners");
    fs::create_dir_all(&owners).expect("owner directory");
    fs::write(
        owners.join("test.yaml"),
        "schema: methexis.owner/v1alpha1\nid: test\n",
    )
    .expect("owner record");
    let sources = root.path().join("methexis/sources/decision");
    fs::create_dir_all(&sources).expect("source directory");
    let mut source = SourceRecord {
        schema: SOURCE_SCHEMA.to_owned(),
        id: "test.source".to_owned(),
        revision: String::new(),
        payload: SourcePayload::Decision {
            content: "Test provenance.".to_owned(),
        },
    };
    source.revision = super::revision::source(&source);
    fs::write(
        sources.join("test.source.yaml"),
        serde_norway::to_string(&source).expect("serialize Source"),
    )
    .expect("source record");
    let path = root.path().join("methexis/knowledge").join(relative);
    fs::create_dir_all(path.parent().expect("record parent")).expect("knowledge directory");
    fs::write(
        path,
        format!(
            "\
---
schema: methexis.knowledge/v1alpha1
id: {id}
kind: rule
owner: test
sources:
  - id: test.source
    revision: {}
relations: {{}}
---
# Test rule

## Statement

The catalog contains a stable test statement.
",
            source.revision
        ),
    )
    .expect("knowledge record");
}

#[test]
fn duplicate_knowledge_id_invalidates_the_whole_catalog() {
    let root = TestDirectory::new("duplicate-id");
    write_knowledge(&root, "one.md", "test.duplicate");
    write_knowledge(&root, "two.md", "test.duplicate");

    let error = load(root.path()).err().expect("catalog must fail");
    let envelope = error.into_envelope();

    assert_eq!(envelope.error.code, "duplicate_knowledge_id");
}

#[test]
fn stale_projection_is_valid_but_not_searchable() {
    let root = TestDirectory::new("stale-projection");
    write_knowledge(&root, "unit.md", "test.projection");
    let projection = root
        .path()
        .join("methexis/review-projections/test.projection.md");
    fs::create_dir_all(projection.parent().expect("projection parent"))
        .expect("projection directory");
    fs::write(
        projection,
        super::projection::fixture_bytes("test.projection", HASH, "오래된번역표식"),
    )
    .expect("stale projection");
    let catalog = load(root.path()).expect("stale projection is structurally valid");
    let result = discovery::discover(
        DiscoveryRequest {
            schema: REQUEST_SCHEMA.to_owned(),
            query: Some("오래된번역표식".to_owned()),
            anchors: Vec::new(),
        },
        &catalog,
    )
    .expect("discovery succeeds");

    assert!(result.candidates.is_empty());
}

#[test]
fn directly_edited_exact_revision_projection_invalidates_catalog() {
    let root = TestDirectory::new("edited-projection");
    write_knowledge(&root, "unit.md", "test.projection");
    let revision = load(root.path()).expect("base catalog").units["test.projection"]
        .revision
        .clone();
    let projection = root
        .path()
        .join("methexis/review-projections/test.projection.md");
    fs::create_dir_all(projection.parent().expect("projection parent"))
        .expect("projection directory");
    let bytes = super::projection::fixture_bytes("test.projection", &revision, "원래 번역");
    let edited = String::from_utf8(bytes)
        .expect("UTF-8 fixture")
        .replace("원래 번역", "직접 수정한 번역");
    fs::write(projection, edited).expect("edited projection");

    let error = load(root.path()).err().expect("catalog must fail");

    assert_eq!(error.into_envelope().error.code, "invalid_catalog_record");
}

#[test]
fn forbidden_html_and_duplicate_sections_invalidate_knowledge() {
    for (label, addition) in [
        ("raw-html", "\n<aside>hidden</aside>\n"),
        ("duplicate-section", "\n## Statement\n\nSecond statement.\n"),
    ] {
        let root = TestDirectory::new(label);
        write_knowledge(&root, "unit.md", "test.invalid");
        let path = root.path().join("methexis/knowledge/unit.md");
        let mut body = fs::read_to_string(&path).expect("knowledge");
        body.push_str(addition);
        fs::write(path, body).expect("invalid knowledge");

        let error = load(root.path()).err().expect("catalog must fail");
        assert_eq!(error.into_envelope().error.code, "invalid_catalog_record");
    }
}

#[test]
fn required_heading_inside_a_fence_does_not_validate_body() {
    let root = TestDirectory::new("fenced-heading");
    write_knowledge(&root, "unit.md", "test.invalid");
    let path = root.path().join("methexis/knowledge/unit.md");
    let body = fs::read_to_string(&path).expect("knowledge").replace(
        "## Statement\n\nThe catalog contains a stable test statement.",
        "```\n## Statement\n\nHidden statement.\n```",
    );
    fs::write(path, body).expect("fenced heading");

    let error = load(root.path()).err().expect("catalog must fail");

    assert_eq!(error.into_envelope().error.code, "invalid_catalog_record");
}

#[test]
fn required_relation_cycle_invalidates_the_whole_catalog() {
    let root = TestDirectory::new("required-cycle");
    write_knowledge(&root, "one.md", "test.one");
    write_knowledge(&root, "two.md", "test.two");
    for (file, target) in [("one.md", "test.two"), ("two.md", "test.one")] {
        let path = root.path().join("methexis/knowledge").join(file);
        let body = fs::read_to_string(&path).expect("knowledge").replace(
            "relations: {}",
            &format!("relations:\n  depends_on:\n    - {target}"),
        );
        fs::write(path, body).expect("cyclic relation");
    }

    let error = load(root.path()).err().expect("catalog must fail");
    let envelope = error.into_envelope();

    assert_eq!(envelope.error.code, "invalid_relation_graph");
    assert_eq!(envelope.error.affected_ids.len(), 3);
}

#[test]
fn supersedes_cycle_invalidates_the_whole_catalog() {
    let root = TestDirectory::new("supersedes-cycle");
    write_knowledge(&root, "one.md", "test.one");
    write_knowledge(&root, "two.md", "test.two");
    for (file, target) in [("one.md", "test.two"), ("two.md", "test.one")] {
        let path = root.path().join("methexis/knowledge").join(file);
        let body = fs::read_to_string(&path).expect("knowledge").replace(
            "relations: {}",
            &format!("relations:\n  supersedes:\n    - {target}"),
        );
        fs::write(path, body).expect("cyclic relation");
    }

    let error = load(root.path()).err().expect("catalog must fail");

    assert_eq!(error.into_envelope().error.code, "invalid_relation_graph");
}

#[test]
fn empty_knowledge_corpus_is_invalid() {
    let root = TestDirectory::new("empty-corpus");
    fs::create_dir_all(root.path().join("methexis/knowledge")).expect("knowledge directory");
    fs::create_dir_all(root.path().join("methexis/owners")).expect("owner directory");
    fs::create_dir_all(root.path().join("methexis/sources")).expect("source directory");

    let error = load(root.path()).err().expect("catalog must fail");

    assert_eq!(error.into_envelope().error.code, "invalid_relation_graph");
}

#[test]
fn missing_source_invalidates_the_whole_catalog() {
    let root = TestDirectory::new("missing-source");
    write_knowledge(&root, "unit.md", "test.source-link");
    fs::remove_file(
        root.path()
            .join("methexis/sources/decision/test.source.yaml"),
    )
    .expect("remove Source");

    let error = load(root.path()).err().expect("catalog must fail");

    assert_eq!(error.into_envelope().error.code, "missing_source");
}

#[test]
fn pinned_older_source_revision_remains_discoverable() {
    let root = TestDirectory::new("stale-source-reference");
    write_knowledge(&root, "unit.md", "test.stale-source");
    let path = root.path().join("methexis/knowledge/unit.md");
    let body = fs::read_to_string(&path).expect("knowledge");
    let source_revision = body
        .lines()
        .find_map(|line| line.trim().strip_prefix("revision: "))
        .expect("SourceRevision");
    let body = body.replace(source_revision, HASH);
    fs::write(path, body).expect("stale Source reference");

    let catalog = load(root.path()).expect("stale eligibility is not structural invalidity");

    assert!(catalog.units.contains_key("test.stale-source"));
}

// 실제 저장소 corpus가 기존 seed와 14개 Surface 계약의 exact revision 및
// 유효한 한국어 Projection을 유지하면서도 후속 확장을 허용하는지 검증한다.
#[test]
fn reference_corpus_matches_methexis_revision_and_projection_contracts() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate is nested below repository root");
    let catalog = load(root).expect("reference corpus is valid");

    // seed는 이미 활성화된 최초 계약이다. Surface corpus가 커지더라도
    // 이 ID와 revision이 사라지거나 바뀌면 기존 지식 기반이 깨진 것이다.
    let expected_seed = BTreeMap::from([
        (
            "tui.architecture.evidence-based-split",
            "sha256:0787fb2d64d3d16201752a02130ea45f9287734f37b6bf10f0269f6b239f8794",
        ),
        (
            "tui.architecture.module-boundaries",
            "sha256:4c2604b602190817b68ceccf6f5e726fadc89b5fb32875dedcc17d53bfa1533e",
        ),
        (
            "tui.crate.ui-only-boundary",
            "sha256:890ea5c07e508f9b29a01edeb5e274fffa4750e3cfe22d1fc1ef53546988d2a4",
        ),
        (
            "tui.dependencies.selection-gate",
            "sha256:11a84ded112fda98eee50b4b6230b3291a51f65791f2370ccbcd8ca7f142a208",
        ),
        (
            "tui.runtime.typed-flow",
            "sha256:191d3c5030c6e2e161556232cd548bccf8b375cb52a85a586db14fb6aa6dac49",
        ),
    ]);

    // Surface corpus 자체는 후속 Slice에서 확장할 수 있다. 다만 지금 검수한
    // 14개 계약은 exact revision과 유효한 한국어 Projection까지 함께 보존해야 한다.
    let expected_surface = BTreeMap::from([
        // grapheme 쓰기는 원자적으로 수행하고, 주변 셀의 재배치는 상위 layout에 맡긴다.
        (
            "tui.surface.atomic-grapheme-write",
            "sha256:c1cdcafa4c92e4e590431b36b8afa3cdeace0e2a9b3355bc544bb76580eaac02",
        ),
        // 비어 있는 셀도 resolved Style을 가진 명시적인 상태로 취급한다.
        (
            "tui.surface.blank-cell",
            "sha256:ec8988e176bf90cbe93c8c0d19c547dbf20fe006e08d79fc89ceb7d052d7ba85",
        ),
        // component는 자신에게 할당된 Rect 안에서만 Surface를 읽고 변경한다.
        (
            "tui.surface.bounded-view",
            "sha256:87ae8d0afee3a38ac35fe33cc9d7edfcbc96809236d6931e1fa22f7bd5fb9634",
        ),
        // 완성된 이전·현재 frame의 차이는 항상 같은 row span 순서로 나온다.
        (
            "tui.surface.deterministic-diff",
            "sha256:269a7815cb3c6b213295b70da7c26ddc2dded7a776bfbe12353d5b2ebff41e4c",
        ),
        // viewport 좌표는 u16과 checked arithmetic으로 안전하게 계산한다.
        (
            "tui.surface.geometry",
            "sha256:41f9fe004f1a95d1d95b6810cd05408e5e356e17858ee3f7f0002164d2abff8f",
        ),
        // wide grapheme은 leader와 역참조 가능한 continuation 셀로 표현한다.
        (
            "tui.surface.grapheme-cells",
            "sha256:f0c1a62e8e1121618003f8b5c264fc77945afb7bec087037813ce2bbba6d72ff",
        ),
        // HTML은 ANSI를 흉내 내지 않고 완성된 Surface를 직접 투영한다.
        (
            "tui.surface.html-projection",
            "sha256:8779702ef532b1b0761c59cb10ba935dac5fe84537f6cb9f10231c27e133cd21",
        ),
        // 기존 wide grapheme과 겹치면 기존 footprint 전체를 원자적으로 정리한다.
        (
            "tui.surface.intersecting-overwrite",
            "sha256:f3a763bf9e406f42fc674a22fbe37e1585074bffb66436854553f48408d2aa0f",
        ),
        // Surface는 완성된 2차원 셀 상태만 소유하고 terminal lifecycle은 소유하지 않는다.
        (
            "tui.surface.model-ownership",
            "sha256:d1529670a39e3d9ca4cda0fcaf822c2afee833043334b1894931f25e821bcd24",
        ),
        // 셀에는 theme 역할이 아니라 최종 계산된 Style을 inline으로 저장한다.
        (
            "tui.surface.resolved-style",
            "sha256:7210c4f0cdb9a5f7382c0e7edb7ea24d40b42181259854d2e4ae558b284ac33e",
        ),
        // terminal 출력은 FrameDiff에서 typed operation을 거쳐 ANSI로 변환한다.
        (
            "tui.surface.terminal-ops",
            "sha256:ad84fe74ecf5998e0f5f20c92ac793d02550bc114b0836d580d916b47d63c1b1",
        ),
        // 문자열은 Unicode 17.0 extended grapheme cluster 단위로 나눈다.
        (
            "tui.surface.text-segmentation",
            "sha256:a4911404c56747266cada0136602123dc0c06115f330f8bd6e7fbd355bdd46f3",
        ),
        // 실제 PTY를 출력 권위로 두고 tmux·SSH 환경의 미검증 상태도 추적한다.
        (
            "tui.surface.validation-matrix",
            "sha256:b50c6d872a02e25a95c7397bf8c46a1f32bbcdeb22d429c49832cffbb9e1bd1d",
        ),
        // terminal과 HTML은 동일한 Unicode 17.0 기반 셀 너비 규칙을 사용한다.
        (
            "tui.surface.width-profile",
            "sha256:6f83deba02e9cce6473c947191acca41e160fe9a78a3a2b9e1646ecd5aac0883",
        ),
    ]);

    // 전체 개수를 고정하면 정상적인 corpus 확장도 실패한다. 따라서 하한만 확인하고,
    // 아래 반복문에서 현재 필수 계약 각각의 정확한 내용을 별도로 고정한다.
    assert!(
        catalog.units.len() >= expected_seed.len() + expected_surface.len(),
        "the extensible reference corpus must retain the seed and Surface contracts"
    );
    for (id, revision) in expected_seed.into_iter().chain(expected_surface) {
        let unit = &catalog.units[id];
        assert_eq!(unit.revision, revision);
        assert!(unit.projection.is_some(), "{id} Projection must be valid");
    }
}
