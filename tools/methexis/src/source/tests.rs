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
fn source_loader_rejects_duplicate_ids_before_context_freshness_mapping() {
    let repository = TemporaryRepository::new();
    let source = write_source(&repository, decision_record("duplicate"));
    let duplicate = source.path.with_file_name("duplicate.yaml");
    fs::copy(&source.path, duplicate).unwrap();

    let diagnostics = super::load(&repository.path).unwrap_err();

    assert_eq!(diagnostics.len(), 2);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "duplicate_source_id")
    );
}

#[test]
fn captured_source_record_rejects_same_semantics_with_new_file_identity() {
    let repository = TemporaryRepository::new();
    let source = write_source(&repository, decision_record("captured"));
    let (_, captures) = super::load_captured(&repository.path).unwrap();
    let bytes = fs::read(&source.path).unwrap();
    let replacement = source.path.with_extension("replacement");
    fs::write(&replacement, bytes).unwrap();
    fs::rename(replacement, &source.path).unwrap();

    let failure =
        super::working_tree::final_revalidate(&repository.path, &captures[0]).unwrap_err();

    assert_eq!(failure.code, "source_changed_during_validation");
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
fn final_code_read_detects_a_mutation_before_its_post_read_stat() {
    let repository = TemporaryRepository::new();
    let path = repository.path.join("source.rs");
    fs::write(&path, b"captured\n").unwrap();
    let capture = match working_tree::capture(&repository.path, "source.rs", &sha256(b"captured\n"))
        .unwrap()
    {
        working_tree::CaptureState::Fresh(capture) => capture,
        _ => panic!("initial bytes should be fresh"),
    };

    let failure = working_tree::final_revalidate_after_read(&repository.path, &capture, || {
        fs::write(&path, b"changed!\n").unwrap();
    })
    .expect_err("post-read stat must detect the concurrent mutation");

    let report = crate::check::failed_authority_report(
        crate::checkpoint::AuthorityFailure::from_source("0123456789abcdef", failure),
    );
    assert!(report.retryable);
    assert!(report.units.is_empty());
    assert_eq!(report.snapshot_revision, None);
    assert_eq!(report.trusted_commit.as_deref(), Some("0123456789abcdef"));
    assert_eq!(
        report.next_actions,
        ["retry `methexis check`; no state was published"]
    );
}

#[test]
fn final_code_read_detects_same_byte_path_replacement() {
    let repository = TemporaryRepository::new();
    let path = repository.path.join("source.rs");
    let displaced = repository.path.join("source.old");
    fs::write(&path, b"captured\n").unwrap();
    let capture = match working_tree::capture(&repository.path, "source.rs", &sha256(b"captured\n"))
        .unwrap()
    {
        working_tree::CaptureState::Fresh(capture) => capture,
        _ => panic!("initial bytes should be fresh"),
    };

    let failure = working_tree::final_revalidate_after_read(&repository.path, &capture, || {
        fs::rename(&path, &displaced).unwrap();
        fs::write(&path, b"captured\n").unwrap();
    })
    .expect_err("post-read path replacement must fail even when bytes match");

    assert_eq!(failure.code, "source_changed_during_validation");
}

#[test]
fn final_code_read_rehashes_when_modeled_identity_is_restored() {
    use std::fs::{FileTimes, OpenOptions};

    let repository = TemporaryRepository::new();
    let path = repository.path.join("source.rs");
    fs::write(&path, b"captured\n").unwrap();
    let metadata = fs::metadata(&path).unwrap();
    let modified = metadata.modified().unwrap();
    let accessed = metadata.accessed().unwrap();
    let capture = match working_tree::capture(&repository.path, "source.rs", &sha256(b"captured\n"))
        .unwrap()
    {
        working_tree::CaptureState::Fresh(capture) => capture,
        _ => panic!("initial bytes should be fresh"),
    };

    let failure = working_tree::final_revalidate_after_read(&repository.path, &capture, || {
        fs::write(&path, b"modified\n").unwrap();
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                FileTimes::new()
                    .set_accessed(accessed)
                    .set_modified(modified),
            )
            .unwrap();
    })
    .expect_err("the final current-path hash must detect restored-metadata byte drift");

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
    let decision = write_source(&repository, decision_record("Accepted."));
    let mut selected_unit = unit("tui.selected", Relations::default());
    selected_unit.metadata.sources = [
        (&record.id, &record.revision),
        (&decision.record.id, &decision.record.revision),
    ]
    .map(|(id, revision)| SourceRef {
        id: id.clone(),
        revision: revision.clone(),
    })
    .to_vec();
    let source = Source {
        record,
        path: source_path,
    };
    let trusted = Foundation {
        units: vec![selected_unit],
        owners: Vec::new(),
        sources: vec![source, decision],
    };
    let working = Foundation {
        units: trusted.units.clone(),
        owners: Vec::new(),
        sources: trusted.sources.clone(),
    };
    let selected = BTreeSet::from(["tui.selected".to_owned()]);

    let fresh = super::evaluate(&repository.path, &trusted, &working.sources, &selected).unwrap();
    assert_eq!(fresh.checkpoint, "active");
    assert_eq!(
        fresh.units["tui.selected"].evidence,
        [
            "code_hash_match:tui.code",
            "decision_revision_match:tui.decision"
        ]
    );
    fs::write(repository.path.join("src/lib.rs"), b"drifted\n").unwrap();
    let drifted = super::evaluate(&repository.path, &trusted, &working.sources, &selected).unwrap();

    assert_eq!(drifted.checkpoint, "degraded");
    assert_eq!(
        drifted.units["tui.selected"].eligibility,
        Eligibility::Stale
    );
}

#[test]
fn conversation_and_external_sources_fail_closed_in_a_multi_source_unit() {
    let repository = TemporaryRepository::new();
    let conversation = write_source(
        &repository,
        SourceRecord {
            schema: SOURCE_SCHEMA.to_owned(),
            id: "tui.conversation".to_owned(),
            revision: hash('0'),
            payload: SourcePayload::Conversation {
                material: crate::model::ConversationMaterial::Excerpt {
                    content: "Authorized excerpt.".to_owned(),
                },
            },
        },
    );
    let external = write_source(
        &repository,
        SourceRecord {
            schema: SOURCE_SCHEMA.to_owned(),
            id: "tui.external".to_owned(),
            revision: hash('0'),
            payload: SourcePayload::External {
                freshness: crate::model::ExternalFreshness::Immutable {
                    locator: "https://example.invalid/spec".to_owned(),
                    version: "v1".to_owned(),
                    content_hash: hash('4'),
                },
            },
        },
    );
    let mut selected_unit = unit("tui.selected", Relations::default());
    selected_unit.metadata.sources = [&conversation, &external]
        .map(|source| SourceRef {
            id: source.record.id.clone(),
            revision: source.record.revision.clone(),
        })
        .to_vec();
    let trusted = foundation(selected_unit, vec![conversation, external]);
    let working = clone_foundation(&trusted);
    let selected = BTreeSet::from(["tui.selected".to_owned()]);

    let evaluation =
        super::evaluate(&repository.path, &trusted, &working.sources, &selected).unwrap();

    assert_eq!(evaluation.checkpoint, "degraded");
    assert_eq!(
        evaluation.units["tui.selected"].evidence,
        [
            "conversation_unverified:tui.conversation",
            "external_unverified:tui.external"
        ]
    );
}

#[test]
fn a_working_decision_change_can_only_demote_trusted_knowledge() {
    let repository = TemporaryRepository::new();
    let trusted_source = source(decision_record("Accepted."));
    let working_source = write_source(&repository, decision_record("Changed."));
    let mut selected_unit = unit("tui.selected", Relations::default());
    selected_unit.metadata.sources = vec![SourceRef {
        id: trusted_source.record.id.clone(),
        revision: trusted_source.record.revision.clone(),
    }];
    let trusted = foundation(selected_unit.clone(), vec![trusted_source]);
    let working = foundation(selected_unit, vec![working_source]);
    let selected = BTreeSet::from(["tui.selected".to_owned()]);

    let evaluation =
        super::evaluate(&repository.path, &trusted, &working.sources, &selected).unwrap();

    assert_eq!(evaluation.checkpoint, "degraded");
    assert_eq!(
        evaluation.units["tui.selected"].evidence,
        ["working_source_drift:tui.decision"]
    );
}

#[test]
fn missing_and_mismatched_trusted_sources_are_distinct_failures() {
    let repository = TemporaryRepository::new();
    let mismatched = write_source(&repository, decision_record("Accepted."));
    let mut selected_unit = unit("tui.selected", Relations::default());
    selected_unit.metadata.sources = vec![
        SourceRef {
            id: mismatched.record.id.clone(),
            revision: hash('9'),
        },
        SourceRef {
            id: "tui.missing".to_owned(),
            revision: hash('8'),
        },
    ];
    let trusted = foundation(selected_unit, vec![mismatched]);
    let working = clone_foundation(&trusted);
    let selected = BTreeSet::from(["tui.selected".to_owned()]);

    let evaluation =
        super::evaluate(&repository.path, &trusted, &working.sources, &selected).unwrap();

    assert_eq!(evaluation.checkpoint, "degraded");
    assert_eq!(
        evaluation.units["tui.selected"].eligibility,
        Eligibility::Invalid
    );
    assert_eq!(
        evaluation.units["tui.selected"].evidence,
        [
            "source_missing:tui.missing",
            "source_revision_mismatch:tui.decision"
        ]
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

fn decision_record(content: &str) -> SourceRecord {
    let mut record = SourceRecord {
        schema: SOURCE_SCHEMA.to_owned(),
        id: "tui.decision".to_owned(),
        revision: hash('0'),
        payload: SourcePayload::Decision {
            content: content.to_owned(),
        },
    };
    record.revision = revision::calculate(&record);
    record
}

fn source(record: SourceRecord) -> Source {
    Source {
        record,
        path: PathBuf::new(),
    }
}

fn write_source(repository: &TemporaryRepository, mut record: SourceRecord) -> Source {
    record.revision = revision::calculate(&record);
    let kind = record.payload.kind();
    let path = repository
        .path
        .join("methexis/sources")
        .join(kind)
        .join(format!("{}.yaml", record.id));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_norway::to_string(&record).unwrap()).unwrap();
    Source { record, path }
}

fn foundation(unit: KnowledgeUnit, sources: Vec<Source>) -> Foundation {
    Foundation {
        units: vec![unit],
        owners: Vec::new(),
        sources,
    }
}

fn clone_foundation(foundation: &Foundation) -> Foundation {
    Foundation {
        units: foundation.units.clone(),
        owners: foundation.owners.clone(),
        sources: foundation.sources.clone(),
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
