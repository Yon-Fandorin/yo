#![allow(unused_qualifications)]

#[path = "tests/freshness.rs"]
mod freshness_tests;
#[path = "tests/negative.rs"]
mod negative_tests;
#[path = "tests/working_tree.rs"]
mod working_tree_tests;

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::ErrorKind,
    path::PathBuf,
    process, slice,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    Eligibility, NegativeRecords, UnitFreshness, evaluate, final_revalidate, load_captured,
    negative, revision, working_tree,
};
use crate::{
    check,
    check::Foundation,
    checkpoint, model,
    model::{
        ExternalFreshness, KnowledgeKind, KnowledgeMetadata, KnowledgeUnit, Owner, Relations,
        SOURCE_SCHEMA, Source, SourcePayload, SourceRecord, SourceRef,
    },
};

static TEMPORARY_REPOSITORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static TEMPORARY_REPOSITORY_NONCE: OnceLock<u128> = OnceLock::new();

// code Source의 line_hint가 달라도 revision은 같게 계산된다.
#[test]
fn source_revision_excludes_code_line_hint() {
    let first = code_record(Some(10));
    let second = code_record(Some(900));

    assert_eq!(revision::calculate(&first), revision::calculate(&second));
}

// 의미상 같은 바이트라도 payload kind가 다르면 서로 다른 revision이 된다.
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
            material: model::ConversationMaterial::Excerpt {
                content: "same semantic bytes".to_owned(),
            },
        },
    };

    assert_ne!(
        revision::calculate(&decision),
        revision::calculate(&conversation)
    );
}

// 닫힌 Source payload schema는 모든 지원 kind를 읽고 알 수 없는 필드는 거부한다.
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

// 같은 SourceId를 가진 두 파일을 하나로 덮어쓰지 않고 source load 전체를 거부한다.
// 어느 파일이 충돌했는지 알 수 있도록 두 경로 모두에 duplicate_source_id 진단을 남긴다.
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

// 여러 근거 중 conversation이나 external Source처럼 아직 검증할 수 없는 종류가 하나라도 섞이면
// 검증된 일부만 보고 전체를 신뢰하지 않고 지식 단위를 안전한 실패 상태로 둔다.
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
                material: model::ConversationMaterial::Excerpt {
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
                freshness: ExternalFreshness::Immutable {
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

// 아직 승인되지 않은 작업 중 decision은 trusted 지식의 권한을 더 높이는 근거가 될 수 없다.
// 현재 신뢰 수준을 유지하거나 변경 영향에 따라 stale·degraded로만 낮출 수 있다.
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

// 참조한 Source 파일이 없는 경우와 기대 revision이 아닌 경우는 모두 지식을 invalid로 만들지만
// 복구 방법은 다르다. 원인을 구분하도록 source_missing과 source_revision_mismatch 증거를 보존한다.
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

// 필수 의존 지식이 stale이면 그 지식을 사용하는 dependent만 stale로 전파한다.
// 관계없는 지식은 active로 유지해 하나의 실패가 catalog 전체를 막지 않게 한다.
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
        negative_records: NegativeRecords::empty(),
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

    for eligibility in [Eligibility::Suspect, Eligibility::Invalid] {
        states.get_mut("tui.dependency").unwrap().eligibility = eligibility;
        states.get_mut("tui.dependent").unwrap().eligibility = Eligibility::Active;
        states.get_mut("tui.dependent").unwrap().evidence.clear();
        super::freshness::propagate_required_dependents(&indexed, &selected, &mut states);
        assert_eq!(states["tui.dependent"].eligibility, eligibility);
        assert_eq!(states["tui.unaffected"].eligibility, Eligibility::Active);
        assert_eq!(
            states["tui.dependent"].evidence,
            [format!(
                "required_knowledge_state:{}:tui.dependency",
                eligibility.as_str()
            )]
        );
    }
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
        negative_records: NegativeRecords::empty(),
    }
}

fn clone_foundation(foundation: &Foundation) -> Foundation {
    Foundation {
        units: foundation.units.clone(),
        owners: foundation.owners.clone(),
        sources: foundation.sources.clone(),
        negative_records: foundation.negative_records.clone(),
    }
}

fn unit(id: &str, relations: Relations) -> KnowledgeUnit {
    KnowledgeUnit {
        metadata: KnowledgeMetadata {
            schema: model::KNOWLEDGE_SCHEMA.to_owned(),
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
        let nonce = TEMPORARY_REPOSITORY_NONCE.get_or_init(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        });
        loop {
            let sequence = TEMPORARY_REPOSITORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "methexis-source-test-{}-{nonce}-{sequence}",
                process::id(),
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    fs::create_dir(path.join("methexis")).unwrap();
                    fs::write(
                        path.join("methexis/negative-records.yaml"),
                        "schema: methexis.negative-records/v1alpha1\nrecords: []\n",
                    )
                    .unwrap();
                    return Self { path };
                },
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {},
                Err(error) => panic!("create temporary repository: {error}"),
            }
        }
    }
}

impl Drop for TemporaryRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
