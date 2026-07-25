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

#[test]
fn reference_corpus_matches_methexis_revision_and_projection_contracts() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate is nested below repository root");
    let catalog = load(root).expect("reference corpus is valid");
    let expected = BTreeMap::from([
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

    assert_eq!(catalog.units.len(), expected.len());
    for (id, revision) in expected {
        let unit = &catalog.units[id];
        assert_eq!(unit.revision, revision);
        assert!(unit.projection.is_some(), "{id} Projection must be valid");
    }
}
