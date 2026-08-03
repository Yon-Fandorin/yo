use std::fs;

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

// 같은 KnowledgeId가 두 번 등장하면 catalog 전체 load가 duplicate_knowledge_id로 실패한다.
#[test]
fn duplicate_knowledge_id_invalidates_the_whole_catalog() {
    let root = TestDirectory::new("duplicate-id");
    write_knowledge(&root, "one.md", "test.duplicate");
    write_knowledge(&root, "two.md", "test.duplicate");

    let error = load(root.path()).err().expect("catalog must fail");
    let envelope = error.into_envelope();

    assert_eq!(envelope.error.code, "duplicate_knowledge_id");
}

// 번역 Projection이 원문의 현재 revision과 맞지 않아도 카탈로그 자체는 읽을 수 있다.
// 다만 오래된 번역이 검색 결과에 노출되지 않도록 discovery candidate에서는 제외한다.
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

// 원문 revision이 맞더라도 생성된 번역 Projection을 사람이 직접 고치면 무결성이 깨진다.
// 수정된 번역을 공식 검색 자료로 오인하지 않도록 invalid_catalog_record로 거부한다.
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

// raw HTML 추가와 필수 section 중복은 모두 지식 레코드를 invalid_catalog_record로 무효화한다.
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

// 필수 heading이 fence 안에 있으면 section으로 인정되지 않아 invalid_catalog_record로 거부한다.
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

// 두 지식이 서로를 필수 의존성으로 가리키면 어느 쪽도 먼저 처리할 수 없다.
// 오류 경로는 시작 id를 끝에 한 번 더 적어 닫힌 순환임을 보여 주며 catalog 전체를 거부한다.
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

// supersedes 관계 순환도 catalog 전체를 invalid_relation_graph로 무효화한다.
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

// 지식 corpus가 비어 있으면 유효한 catalog가 성립하지 않아 invalid_relation_graph로 실패한다.
#[test]
fn empty_knowledge_corpus_is_invalid() {
    let root = TestDirectory::new("empty-corpus");
    fs::create_dir_all(root.path().join("methexis/knowledge")).expect("knowledge directory");
    fs::create_dir_all(root.path().join("methexis/owners")).expect("owner directory");
    fs::create_dir_all(root.path().join("methexis/sources")).expect("source directory");

    let error = load(root.path()).err().expect("catalog must fail");

    assert_eq!(error.into_envelope().error.code, "invalid_relation_graph");
}

// 지식이 참조하는 Source가 삭제되면 catalog 전체 load가 missing_source로 실패한다.
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

// 지식이 가리키는 SourceRevision이 현재 Source 레코드와 달라도 Librarian은 이를 구조 오류로
// 단정하지 않는다. 권한·freshness 판정은 Methexis에 맡기고 해당 지식은 catalog에 유지한다.
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
