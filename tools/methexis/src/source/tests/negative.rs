use super::*;

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
        slice::from_ref(&external),
        &selected,
    )
    .unwrap();
    assert_eq!(stale.units["tui.selected"].eligibility, Eligibility::Stale);

    write_negative_records(&repository, &hash('b'), &["invalid"]);
    let historical = super::evaluate(
        &repository.path,
        &trusted,
        slice::from_ref(&external),
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
        slice::from_ref(&external),
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
        slice::from_ref(&external),
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
