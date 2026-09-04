use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use super::{Eligibility, NegativeRecords, UnitFreshness, revision, working_tree};
use crate::{
    check::Foundation,
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

// 캡처한 뒤 파일이 다른 파일로 교체되면 내용이 우연히 같아도 같은 입력이라고 단정할 수 없다.
// 바이트뿐 아니라 파일 identity도 비교해 오래된 Source snapshot을 거부한다.
#[test]
fn captured_source_record_rejects_same_semantics_with_new_file_identity() {
    let repository = TemporaryRepository::new();
    let source = write_source(&repository, decision_record("captured"));
    let (_, captures) = super::load_captured(&repository.path).unwrap();
    let bytes = fs::read(&source.path).unwrap();
    let replacement = source.path.with_extension("replacement");
    fs::write(&replacement, bytes).unwrap();
    fs::rename(replacement, &source.path).unwrap();

    let failure = working_tree::final_revalidate(&repository.path, &captures[0]).unwrap_err();

    assert_eq!(failure.code, "source_changed_during_validation");
}

// code Source의 content_hash는 경로나 텍스트 해석이 아니라 디스크에서 읽은 정확한 바이트와
// 일치해야 한다. 읽기를 마친 뒤에도 같은 파일인지 다시 확인해 중간 교체를 놓치지 않는다.
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

// 파일을 다 읽은 직후 최종 상태를 확인하기 전에 내용이 바뀌는 짧은 경주 구간도 닫아야 한다.
// post-read stat이 달라지면 완성된 캡처를 내보내지 않고 동시 변경으로 판정한다.
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

// 읽는 도중 파일을 교체한 뒤 같은 경로와 바이트로 되돌려 놓아도 변경 사실을 숨길 수 없어야 한다.
// 파일 identity 변화를 이용해 이런 교체를 동시 변경으로 검출한다.
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

// 모델상 파일 identity가 다시 원래 값처럼 보여도 이전 hash를 그대로 믿지 않는다.
// 최종 바이트를 다시 읽어 hash함으로써 identity 검사만으로 놓칠 수 있는 내용 변경을 잡는다.
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

// 캡처 시 없던 code 파일이 최종 검증 전에 생기면 동시 변경으로 검출한다.
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

// 승인 뒤 code 파일 바이트가 달라지면 그 승인이 갑자기 다른 권한으로 바뀌어서는 안 된다.
// 기존 권한은 유지하되 해당 code를 사용하는 지식만 degraded 상태로 낮춘다.
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
        negative_records: NegativeRecords::empty(),
    };
    let working = Foundation {
        units: trusted.units.clone(),
        owners: Vec::new(),
        sources: trusted.sources.clone(),
        negative_records: NegativeRecords::empty(),
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

#[cfg(unix)]
// code Source 경로의 어느 구성 요소든 symlink면 실제 파일을 읽지 않고 캡처를 거부한다.
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

// 빈 manifest는 "검사했고 현재 negative record가 없음"을 뜻한다. 반면 파일이 없거나
// 닫힌 schema가 깨지면 검사하지 못한 입력이므로 같은 상태로 간주하지 않고 실패한다.
#[test]
fn negative_record_manifest_distinguishes_evaluated_empty_from_unavailable_or_invalid() {
    let repository = TemporaryRepository::new();

    assert!(super::negative::load(&repository.path).is_ok());

    fs::remove_file(repository.path.join("methexis/negative-records.yaml")).unwrap();
    let missing = super::negative::load(&repository.path).unwrap_err();
    assert_eq!(missing[0].code, "negative_records_unavailable");

    fs::write(
        repository.path.join("methexis/negative-records.yaml"),
        "schema: methexis.negative-records/v1alpha1\nrecords: []\nunknown: true\n",
    )
    .unwrap();
    let invalid = super::negative::load(&repository.path).unwrap_err();
    assert_eq!(invalid[0].code, "invalid_yaml");

    let record = format!(
        "  - knowledge_id: tui.selected\n    revision: {}\n    condition: suspect\n    recorded_by: owner\n    evidence:\n      code: review.hold\n      reference: test://duplicate\n",
        hash('a')
    );
    fs::write(
        repository.path.join("methexis/negative-records.yaml"),
        format!("schema: methexis.negative-records/v1alpha1\nrecords:\n{record}{record}"),
    )
    .unwrap();
    let duplicate = super::negative::load(&repository.path).unwrap_err();
    assert!(
        duplicate
            .iter()
            .any(|diagnostic| diagnostic.code == "duplicate_negative_record")
    );

    fs::write(
        repository.path.join("methexis/negative-records.yaml"),
        format!(
            "schema: methexis.negative-records/v1alpha1\nrecords:\n  - knowledge_id: tui.selected\n    revision: {}\n    condition: suspect\n    recorded_by: owner\n    evidence:\n      code: review.hold\n      reference: test://order/suspect\n  - knowledge_id: tui.selected\n    revision: {}\n    condition: invalid\n    recorded_by: owner\n    evidence:\n      code: review.hold\n      reference: test://order/invalid\n",
            hash('a'),
            hash('a')
        ),
    )
    .unwrap();
    let noncanonical = super::negative::load(&repository.path).unwrap_err();
    assert!(
        noncanonical
            .iter()
            .any(|diagnostic| diagnostic.code == "noncanonical_negative_record_order")
    );

    fs::remove_file(repository.path.join("methexis/negative-records.yaml")).unwrap();
    fs::create_dir(repository.path.join("methexis/negative-records.yaml")).unwrap();
    let unreadable = super::negative::load(&repository.path).unwrap_err();
    assert_eq!(unreadable[0].code, "negative_records_unavailable");
}

#[cfg(unix)]
// manifest는 권한 입력이므로 symlink를 따라 외부나 교체 가능한 별도 파일을 읽지 않는다.
#[test]
fn negative_record_manifest_rejects_symlinks() {
    use std::os::unix::fs::symlink;

    let repository = TemporaryRepository::new();
    let manifest = repository.path.join("methexis/negative-records.yaml");
    let target = repository.path.join("negative-record-target.yaml");
    fs::write(
        &target,
        "schema: methexis.negative-records/v1alpha1\nrecords: []\n",
    )
    .unwrap();
    fs::remove_file(&manifest).unwrap();
    symlink(&target, &manifest).unwrap();

    let diagnostics = super::negative::load(&repository.path).unwrap_err();
    assert_eq!(diagnostics[0].code, "negative_records_unavailable");
}

// 평가 뒤 같은 바이트로 파일만 교체해도 처음 읽은 권한 입력과 같은 객체가 아니다.
// 최종 guard는 의미 비교만 하지 않고 캡처한 identity 변화까지 retryable 실패로 돌린다.
#[test]
fn negative_record_guard_detects_same_byte_identity_replacement() {
    let repository = TemporaryRepository::new();
    fs::create_dir(repository.path.join("methexis/sources")).unwrap();
    let trusted = Foundation {
        units: Vec::new(),
        owners: Vec::new(),
        sources: Vec::new(),
        negative_records: NegativeRecords::empty(),
    };
    let evaluation = super::evaluate(&repository.path, &trusted, &[], &BTreeSet::new()).unwrap();
    let manifest = repository.path.join("methexis/negative-records.yaml");
    let bytes = fs::read(&manifest).unwrap();
    fs::remove_file(&manifest).unwrap();
    fs::write(&manifest, bytes).unwrap();

    let failure = super::final_revalidate(&repository.path, &evaluation.guard).unwrap_err();

    assert_eq!(failure.code, "negative_records_changed_during_validation");
}

// negative record는 Source 상태보다 강한 순서로 exact KU revision에만 적용된다.
// trusted record는 working tree에서 지워도 해제되지 않고, working 추가는 demotion만 만든다.
#[test]
fn exact_negative_records_apply_invalid_over_suspect_over_stale_and_cannot_be_locally_cleared() {
    let repository = TemporaryRepository::new();
    let external = write_source(
        &repository,
        SourceRecord {
            schema: SOURCE_SCHEMA.to_owned(),
            id: "tui.external".to_owned(),
            revision: hash('0'),
            payload: SourcePayload::External {
                freshness: ExternalFreshness::Immutable {
                    locator: "https://example.invalid/evidence".to_owned(),
                    version: "v1".to_owned(),
                    content_hash: hash('9'),
                },
            },
        },
    );
    let mut selected_unit = unit("tui.selected", Relations::default());
    selected_unit.metadata.sources = vec![SourceRef {
        id: external.record.id.clone(),
        revision: external.record.revision.clone(),
    }];
    let revision = selected_unit.revision.clone();
    let mut trusted = foundation(selected_unit, vec![external.clone()]);
    trusted.owners.push(Owner {
        id: "owner".to_owned(),
        path: PathBuf::new(),
    });
    let selected = BTreeSet::from(["tui.selected".to_owned()]);

    let stale = super::evaluate(
        &repository.path,
        &trusted,
        std::slice::from_ref(&external),
        &selected,
    )
    .unwrap();
    assert_eq!(stale.units["tui.selected"].eligibility, Eligibility::Stale);

    write_negative_records(&repository, &hash('b'), &["invalid"]);
    let historical = super::evaluate(
        &repository.path,
        &trusted,
        std::slice::from_ref(&external),
        &selected,
    )
    .unwrap();
    assert_eq!(
        historical.units["tui.selected"].eligibility,
        Eligibility::Stale
    );

    write_negative_records(&repository, &revision, &["suspect"]);
    let suspect = super::evaluate(
        &repository.path,
        &trusted,
        std::slice::from_ref(&external),
        &selected,
    )
    .unwrap();
    assert_eq!(
        suspect.units["tui.selected"].eligibility,
        Eligibility::Suspect
    );
    assert!(
        suspect.units["tui.selected"]
            .evidence
            .iter()
            .any(|evidence| evidence.starts_with("negative_record:working:suspect:sha256:"))
    );

    trusted.negative_records = super::negative::load(&repository.path).unwrap();
    write_negative_records(&repository, &revision, &[]);
    let retained = super::evaluate(
        &repository.path,
        &trusted,
        std::slice::from_ref(&external),
        &selected,
    )
    .unwrap();
    assert_eq!(
        retained.units["tui.selected"].eligibility,
        Eligibility::Suspect
    );
    assert!(
        retained.units["tui.selected"]
            .evidence
            .iter()
            .any(|evidence| evidence.starts_with("negative_record:trusted:suspect:sha256:"))
    );

    write_negative_records(&repository, &revision, &["invalid", "suspect"]);
    let invalid = super::evaluate(&repository.path, &trusted, &[external], &selected).unwrap();
    assert_eq!(
        invalid.units["tui.selected"].eligibility,
        Eligibility::Invalid
    );
}

fn write_negative_records(repository: &TemporaryRepository, revision: &str, conditions: &[&str]) {
    let mut yaml = String::from("schema: methexis.negative-records/v1alpha1\nrecords:");
    if conditions.is_empty() {
        yaml.push_str(" []\n");
    } else {
        yaml.push('\n');
        for condition in conditions {
            yaml.push_str(&format!(
                "  - knowledge_id: tui.selected\n    revision: {revision}\n    condition: {condition}\n    recorded_by: owner\n    evidence:\n      code: review.hold\n      reference: test://negative-record/{condition}\n"
            ));
        }
    }
    fs::write(repository.path.join("methexis/negative-records.yaml"), yaml).unwrap();
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
        let nonce = TEMPORARY_REPOSITORY_NONCE.get_or_init(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        });
        loop {
            let sequence = TEMPORARY_REPOSITORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "methexis-source-test-{}-{nonce}-{sequence}",
                std::process::id(),
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
