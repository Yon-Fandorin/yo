use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{Eligibility, UnitFreshness, revision, working_tree};
use crate::{
    check::Foundation,
    model::{
        KnowledgeKind, KnowledgeMetadata, KnowledgeUnit, Relations, SOURCE_SCHEMA, Source,
        SourcePayload, SourceRecord, SourceRef,
    },
};

#[test]
fn source_revision_excludes_code_line_hint() {
    let first = code_record(Some(10));
    let second = code_record(Some(900));

    assert_eq!(revision::calculate(&first), revision::calculate(&second));
}

#[test]
fn source_revision_is_domain_separated_by_kind() {
    let decision = SourceRecord {
        schema: SOURCE_SCHEMA.to_owned(),
        id: "tui.source".to_owned(),
        revision: hash('0'),
        payload: SourcePayload::Decision {
            content: "same semantic bytes".to_owned(),
        },
    };
    let conversation = SourceRecord {
        schema: SOURCE_SCHEMA.to_owned(),
        id: "tui.source".to_owned(),
        revision: hash('0'),
        payload: SourcePayload::Conversation {
            material: crate::model::ConversationMaterial::Excerpt {
                content: "same semantic bytes".to_owned(),
            },
        },
    };

    assert_ne!(
        revision::calculate(&decision),
        revision::calculate(&conversation)
    );
}

#[test]
fn closed_payload_schema_parses_all_kinds_and_rejects_unknown_fields() {
    for yaml in [
        "\
schema: methexis.source/v1alpha1
id: tui.decision
revision: sha256:0000000000000000000000000000000000000000000000000000000000000000
payload:
  kind: decision
  content: Accepted.
",
        "\
schema: methexis.source/v1alpha1
id: tui.code
revision: sha256:0000000000000000000000000000000000000000000000000000000000000000
payload:
  kind: code
  path: src/lib.rs
  symbol: Surface
  content_hash: sha256:1111111111111111111111111111111111111111111111111111111111111111
",
        "\
schema: methexis.source/v1alpha1
id: tui.conversation
revision: sha256:0000000000000000000000000000000000000000000000000000000000000000
payload:
  kind: conversation
  material:
    mode: opaque
    reference: local:authorized
    content_hash: sha256:2222222222222222222222222222222222222222222222222222222222222222
",
        "\
schema: methexis.source/v1alpha1
id: tui.external
revision: sha256:0000000000000000000000000000000000000000000000000000000000000000
payload:
  kind: external
  freshness:
    freshness: immutable
    locator: https://example.invalid/spec
    version: v1
    content_hash: sha256:3333333333333333333333333333333333333333333333333333333333333333
",
    ] {
        serde_norway::from_str::<SourceRecord>(yaml).expect("closed Source kind parses");
    }

    let unknown = "\
schema: methexis.source/v1alpha1
id: tui.decision
revision: sha256:0000000000000000000000000000000000000000000000000000000000000000
payload:
  kind: decision
  content: Accepted.
  unexpected: true
";
    assert!(serde_norway::from_str::<SourceRecord>(unknown).is_err());
}

#[test]
fn code_capture_hashes_exact_bytes_and_revalidates_identity() {
    let repository = TemporaryRepository::new();
    fs::create_dir(repository.path.join("src")).unwrap();
    fs::write(repository.path.join("src/lib.rs"), b"first\n").unwrap();
    let expected = sha256(b"first\n");

    let capture = match working_tree::capture(&repository.path, "src/lib.rs", &expected).unwrap() {
        working_tree::CaptureState::Fresh(capture) => capture,
        _ => panic!("exact bytes should be fresh"),
    };
    fs::write(repository.path.join("src/lib.rs"), b"first\r\n").unwrap();

    let failure = working_tree::final_revalidate(&repository.path, &capture)
        .expect_err("a later byte change must fail");
    assert_eq!(failure.code, "source_changed_during_validation");
}

#[test]
fn missing_code_capture_detects_a_file_that_appears() {
    let repository = TemporaryRepository::new();
    let capture =
        match working_tree::capture(&repository.path, "appeared.rs", &sha256(b"appeared\n"))
            .unwrap()
        {
            working_tree::CaptureState::Stale { capture, .. } => capture,
            _ => panic!("the initial missing observation must be stale"),
        };
    fs::write(repository.path.join("appeared.rs"), b"appeared\n").unwrap();

    let failure = working_tree::final_revalidate(&repository.path, &capture)
        .expect_err("appearance during validation must be retryable");
    assert_eq!(failure.code, "source_changed_during_validation");
}

#[test]
fn code_drift_degrades_the_selected_unit_without_changing_authority() {
    let repository = TemporaryRepository::new();
    fs::create_dir_all(repository.path.join("src")).unwrap();
    fs::create_dir_all(repository.path.join("methexis/sources/code")).unwrap();
    fs::write(repository.path.join("src/lib.rs"), b"trusted\n").unwrap();
    let mut record = code_record(None);
    let SourcePayload::Code { content_hash, .. } = &mut record.payload else {
        unreachable!()
    };
    *content_hash = sha256(b"trusted\n");
    record.revision = revision::calculate(&record);
    let source_path = repository.path.join("methexis/sources/code/tui.code.yaml");
    fs::write(&source_path, serde_norway::to_string(&record).unwrap()).unwrap();
    let mut selected_unit = unit("tui.selected", Relations::default());
    selected_unit.metadata.sources = vec![SourceRef {
        id: record.id.clone(),
        revision: record.revision.clone(),
    }];
    let source = Source {
        record,
        path: source_path,
    };
    let trusted = Foundation {
        units: vec![selected_unit],
        owners: Vec::new(),
        sources: vec![source],
    };
    let working = Foundation {
        units: trusted.units.clone(),
        owners: Vec::new(),
        sources: trusted.sources.clone(),
    };
    let selected = BTreeSet::from(["tui.selected".to_owned()]);

    let fresh = super::evaluate(&repository.path, &trusted, &working, &selected).unwrap();
    assert_eq!(fresh.checkpoint, "active");
    fs::write(repository.path.join("src/lib.rs"), b"drifted\n").unwrap();
    let drifted = super::evaluate(&repository.path, &trusted, &working, &selected).unwrap();

    assert_eq!(drifted.checkpoint, "degraded");
    assert_eq!(
        drifted.units["tui.selected"].eligibility,
        Eligibility::Stale
    );
}

#[cfg(unix)]
#[test]
fn code_capture_rejects_symlinked_components() {
    use std::os::unix::fs::symlink;

    let repository = TemporaryRepository::new();
    let outside = repository.path.join("outside.rs");
    fs::write(&outside, b"outside\n").unwrap();
    symlink(&outside, repository.path.join("linked.rs")).unwrap();

    let capture = match working_tree::capture(&repository.path, "linked.rs", &sha256(b"outside\n"))
        .unwrap()
    {
        working_tree::CaptureState::Invalid {
            reason: "code_source_path_invalid",
            capture,
        } => capture,
        _ => panic!("symlink must be invalid"),
    };
    fs::remove_file(repository.path.join("linked.rs")).unwrap();
    let failure = working_tree::final_revalidate(&repository.path, &capture)
        .expect_err("an invalid path changing during validation must be retryable");
    assert_eq!(failure.code, "source_changed_during_validation");
}

#[test]
fn stale_required_source_propagates_only_to_dependents() {
    let dependency = unit("tui.dependency", Relations::default());
    let dependent = unit(
        "tui.dependent",
        Relations {
            depends_on: vec!["tui.dependency".to_owned()],
            ..Relations::default()
        },
    );
    let unaffected = unit("tui.unaffected", Relations::default());
    let foundation = Foundation {
        units: vec![dependency, dependent, unaffected],
        owners: Vec::new(),
        sources: Vec::new(),
    };
    let indexed = foundation
        .units
        .iter()
        .map(|unit| (unit.metadata.id.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let selected = foundation
        .units
        .iter()
        .map(|unit| unit.metadata.id.clone())
        .collect::<BTreeSet<_>>();
    let mut states = BTreeMap::from([
        (
            "tui.dependency".to_owned(),
            UnitFreshness {
                eligibility: Eligibility::Stale,
                evidence: vec!["code_hash_mismatch:tui.code".to_owned()],
            },
        ),
        (
            "tui.dependent".to_owned(),
            UnitFreshness {
                eligibility: Eligibility::Active,
                evidence: Vec::new(),
            },
        ),
        (
            "tui.unaffected".to_owned(),
            UnitFreshness {
                eligibility: Eligibility::Active,
                evidence: Vec::new(),
            },
        ),
    ]);

    super::freshness::propagate_required_dependents(&indexed, &selected, &mut states);

    assert_eq!(states["tui.dependent"].eligibility, Eligibility::Stale);
    assert_eq!(states["tui.unaffected"].eligibility, Eligibility::Active);
}

fn code_record(line_hint: Option<u64>) -> SourceRecord {
    SourceRecord {
        schema: SOURCE_SCHEMA.to_owned(),
        id: "tui.code".to_owned(),
        revision: hash('0'),
        payload: SourcePayload::Code {
            path: "src/lib.rs".to_owned(),
            symbol: "Surface".to_owned(),
            content_hash: hash('1'),
            line_hint,
        },
    }
}

fn unit(id: &str, relations: Relations) -> KnowledgeUnit {
    KnowledgeUnit {
        metadata: KnowledgeMetadata {
            schema: crate::model::KNOWLEDGE_SCHEMA.to_owned(),
            id: id.to_owned(),
            kind: KnowledgeKind::Rule,
            owner: "owner".to_owned(),
            sources: vec![SourceRef {
                id: "tui.source".to_owned(),
                revision: hash('2'),
            }],
            relations,
        },
        body: "## Statement\n\nTest.\n".to_owned(),
        path: PathBuf::from(format!("{id}.md")),
        revision: hash('3'),
    }
}

fn hash(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut output = String::from("sha256:");
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

struct TemporaryRepository {
    path: PathBuf,
}

impl TemporaryRepository {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "methexis-source-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }
}

impl Drop for TemporaryRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
