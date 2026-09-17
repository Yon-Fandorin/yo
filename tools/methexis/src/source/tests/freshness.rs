use super::*;

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
