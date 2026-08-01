use super::*;

// v1 와이어 형식이 버전, 세션, 순번, 종류와 독립 계산한 고정 checksum 답안을 남겨
// 향후 구현 변경이 같은 코드로 기대값까지 다시 계산해 오류를 숨기지 않게 합니다.
#[test]
fn writes_an_explicit_versioned_jsonl_envelope() {
    let directory = TestDirectory::new("wire");
    let session_id = session(12);
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
    repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::snapshot("state")),
        )
        .expect("record is written");

    let contents =
        fs::read_to_string(log_path(directory.path(), session_id)).expect("log is readable");
    let value: serde_json::Value = serde_json::from_str(contents.trim()).expect("valid JSON");

    assert_eq!(value["schema"], "yo.session-record/v1");
    assert_eq!(value["session_id"], session_id.to_string());
    assert_eq!(value["sequence"], 1);
    assert_eq!(value["kind"], "snapshot");
    assert_eq!(value["payload"], "state");
    assert_eq!(
        value["discovery"]["descriptor"]["session_id"],
        session_id.to_string()
    );
    assert_eq!(
        value["discovery"]["descriptor"]["workspace_path"]["value"],
        "/workspace"
    );
    assert!(value["discovery"]["updated_unix_millis"].as_u64().is_some());
    assert!(value["discovery"]["binding_epoch"].is_null());
    assert!(value["discovery"]["continuation_anchor_journal_sequence"].is_null());
    assert_eq!(value["checksum"]["schema"], "crc32c/v1");
    assert_eq!(
        value["checksum"]["value"]
            .as_str()
            .expect("checksum is text")
            .len(),
        8
    );
}

// v2와 v3는 공개 호환 형식이 아니라 개발 중간 산출물이므로 새 v1 reader가 이력을
// 암묵적으로 떠안지 않고 unsupported schema로 거부해야 합니다.
#[test]
fn rejects_pre_release_session_record_versions() {
    let directory = TestDirectory::new("pre-release-schema");
    let session_id = session(13);
    let path = log_path(directory.path(), session_id);
    for schema in ["yo.session-record/v2", "yo.session-record/v3"] {
        fs::write(
            &path,
            format!(
                "{{\"schema\":\"{schema}\",\"session_id\":\"{session_id}\",\"sequence\":1,\"kind\":\"incremental\",\"payload\":\"old\"}}\n"
            ),
        )
        .expect("the pre-release fixture is written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("fixture permissions remain restricted");
        let repository =
            LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");

        let error = repository
            .read_after(session_id, None, 8)
            .expect_err("pre-release schema is unsupported");

        assert!(
            error
                .to_string()
                .contains("unsupported Session record schema")
        );
    }
}

// 새 v1은 UUIDv7 Session만 정식 identity로 인정하므로 과거 숫자 표현을 같은 schema로
// 위장한 record를 복구하지 않아야 합니다.
#[test]
fn rejects_a_numeric_session_identity_in_v1() {
    let directory = TestDirectory::new("numeric-v1");
    let session_id = session(14);
    let path = log_path(directory.path(), session_id);
    fs::write(
        &path,
        b"{\"schema\":\"yo.session-record/v1\",\"session_id\":14,\"sequence\":1,\"kind\":\"incremental\",\"payload\":\"invalid\"}\n",
    )
    .expect("the invalid fixture is written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("fixture permissions remain restricted");
    let repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");

    let error = repository
        .read_after(session_id, None, 8)
        .expect_err("numeric Session identity is unsupported");

    assert!(
        error
            .to_string()
            .contains("expected a formatted UUID string"),
        "{error}"
    );
}

// 새 physical v1과 schema 문자열만 같고 discovery가 없는 직전 개발 형식은 현재
// 레코드로 오인하지 않고 닫힌 shape 검사에서 거부해야 합니다.
#[test]
fn rejects_the_displaced_summary_less_v1_shape() {
    let directory = TestDirectory::new("summary-less-v1");
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
        .expect("repository permissions are restricted");
    let session_id = session(24);
    let path = log_path(directory.path(), session_id);
    fs::write(
        &path,
        format!(
            "{{\"schema\":\"yo.session-record/v1\",\"session_id\":\"{session_id}\",\"sequence\":1,\"kind\":\"incremental\",\"payload\":\"old\",\"journal_sequence\":null,\"checksum\":null}}\n"
        ),
    )
    .expect("the displaced fixture is written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("fixture permissions are restricted");
    let reader = LocalSessionReader::open(directory.path()).expect("reader opens existing storage");

    let sessions = reader
        .discover()
        .expect("one corrupt Session does not abort repository discovery");

    assert_eq!(sessions.len(), 1);
    assert!(
        sessions[0]
            .unavailable_reason()
            .is_some_and(|reason| reason.to_string().contains("missing field `discovery`"))
    );
}

// discovery는 앞쪽 레코드를 훑지 않고 마지막 완결 envelope만 검증해 목록을 만들되,
// 전체 history 읽기는 앞쪽 complete-line 손상을 계속 명시적으로 보고해야 합니다.
#[test]
fn discovers_from_the_valid_tail_without_hiding_history_corruption() {
    let directory = TestDirectory::new("bounded-discovery");
    let session_id = session(25);
    {
        let mut repository =
            LocalSessionRepository::open(directory.path(), 32_768).expect("writer opens");
        repository
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("first")),
            )
            .expect("first record is durable");
        repository
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("second")),
            )
            .expect("tail record is durable");
    }
    let path = log_path(directory.path(), session_id);
    let contents = fs::read_to_string(&path).expect("log is readable");
    fs::write(&path, contents.replacen("first", "other", 1)).expect("prefix is corrupted");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("fixture permissions remain restricted");
    let reader = LocalSessionReader::open(directory.path()).expect("reader opens");

    let summaries = reader
        .discover()
        .expect("valid tail discovery remains bounded");
    let history_error = reader
        .read_after(session_id, None, 8)
        .expect_err("history replay validates the complete prefix");

    assert_eq!(summaries.len(), 1);
    assert_eq!(
        summaries[0]
            .summary()
            .expect("tail summary is available")
            .repository_sequence(),
        RepositorySequence::new(2)
    );
    assert!(history_error.to_string().contains("CRC32C"));
}

// writer가 pending marker로 보호한 append 중에는 reader가 writer lease를 빼앗지 않고
// marker의 이전 durable cutoff까지만 읽어 진행 중 바이트를 노출하지 않아야 합니다.
#[test]
fn active_append_exposes_the_previous_durable_snapshot() {
    let directory = TestDirectory::new("active-pending-snapshot");
    let session_id = session(26);
    let mut writer =
        LocalSessionRepository::open(directory.path(), 32_768).expect("writer owns the lease");
    writer
        .append(
            session_id,
            discovered(session_id, DurableRecord::incremental("durable")),
        )
        .expect("prefix is durable");
    let path = log_path(directory.path(), session_id);
    let cutoff = fs::metadata(&path).expect("log exists").len();
    let pending = path.with_extension("jsonl.pending");
    fs::write(&pending, format!("{cutoff}\n")).expect("active marker is simulated");
    fs::set_permissions(&pending, fs::Permissions::from_mode(0o600))
        .expect("marker permissions are restricted");
    OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("log opens")
        .write_all(b"{guarded bytes}\n")
        .expect("guarded bytes are simulated");
    let reader = LocalSessionReader::open(directory.path()).expect("reader opens without a lease");

    let summaries = reader
        .discover()
        .expect("reader uses the pre-append cutoff");
    let history = reader
        .read_after(session_id, None, 8)
        .expect("the durable prefix remains readable");
    let competing_writer = LocalSessionRepository::open(directory.path(), 32_768)
        .expect_err("the reader probe must not release the live writer lock");

    assert_eq!(
        summaries[0]
            .summary()
            .expect("pre-append summary remains available")
            .repository_sequence(),
        RepositorySequence::new(1)
    );
    assert_eq!(history.len(), 1);
    assert!(competing_writer.to_string().contains("another writer owns"));
    drop(writer);
}

// active writer가 없는데 pending marker가 남아 있으면 이전 summary가 있어도 안전하다고
// 추정하지 않고 해당 Session을 unavailable로 분류해야 합니다.
#[test]
fn abandoned_append_marker_quarantines_discovery() {
    let directory = TestDirectory::new("abandoned-pending-discovery");
    let session_id = session(27);
    {
        let mut writer =
            LocalSessionRepository::open(directory.path(), 32_768).expect("writer opens");
        writer
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("durable")),
            )
            .expect("prefix is durable");
    }
    let path = log_path(directory.path(), session_id);
    let cutoff = fs::metadata(&path).expect("log exists").len();
    let pending = path.with_extension("jsonl.pending");
    fs::write(&pending, format!("{cutoff}\n")).expect("abandoned marker is written");
    fs::set_permissions(&pending, fs::Permissions::from_mode(0o600))
        .expect("marker permissions are restricted");
    let reader = LocalSessionReader::open(directory.path()).expect("reader opens");

    let sessions = reader
        .discover()
        .expect("repository listing remains available");

    assert!(matches!(
        sessions[0].unavailable_reason(),
        Some(StoredSessionUnavailableReason::Quarantined { .. })
    ));
}

// 이전 writer가 남긴 marker가 있으면 다음 writer가 같은 root lock을 잡아 그 marker를
// 자기 진행 중 append처럼 보이게 하지 못하고 저장소 전체를 먼저 격리해야 합니다.
#[test]
fn successor_writer_cannot_coexist_with_an_abandoned_marker() {
    let directory = TestDirectory::new("successor-with-abandoned-marker");
    let session_id = session(31);
    {
        let mut writer =
            LocalSessionRepository::open(directory.path(), 32_768).expect("first writer opens");
        writer
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("durable")),
            )
            .expect("the durable prefix is written");
    }
    let pending = log_path(directory.path(), session_id).with_extension("jsonl.pending");
    fs::write(&pending, b"0\n").expect("an abandoned marker is simulated");
    fs::set_permissions(&pending, fs::Permissions::from_mode(0o600))
        .expect("marker permissions are restricted");

    let error = LocalSessionRepository::open(directory.path(), 32_768)
        .expect_err("a successor writer cannot adopt an old marker");

    assert!(matches!(
        error,
        crate::session_repository::RepositoryError::Quarantined { .. }
    ));
}

// newline 하나는 미완결 tail이 아니라 완결되었지만 JSON이 아닌 한 줄이므로 discovery가
// 빈 Session으로 낮추지 않고 typed corruption으로 보고해야 합니다.
#[test]
fn complete_empty_line_is_reported_as_tail_corruption() {
    let directory = TestDirectory::new("complete-empty-line");
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
        .expect("repository permissions are restricted");
    let session_id = session(32);
    let path = log_path(directory.path(), session_id);
    fs::write(&path, b"\n").expect("one complete invalid line is written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("fixture permissions are restricted");
    let reader = LocalSessionReader::open(directory.path()).expect("reader opens");

    let sessions = reader.discover().expect("listing remains available");

    assert!(matches!(
        sessions[0].unavailable_reason(),
        Some(StoredSessionUnavailableReason::Corrupt { .. })
    ));
}

// 저장소 root의 관련 없는 jsonl 파일은 정상 UUIDv7 Session 하나를 찾는 전체 목록을
// 중단하지 않아 도구나 사용자가 남긴 낯선 파일 하나가 모든 history를 숨기지 않습니다.
#[test]
fn unrelated_jsonl_filename_does_not_abort_discovery() {
    let directory = TestDirectory::new("unrelated-jsonl");
    let session_id = session(33);
    {
        let mut writer =
            LocalSessionRepository::open(directory.path(), 32_768).expect("writer opens");
        writer
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("durable")),
            )
            .expect("the Session is durable");
    }
    let unrelated = directory.path().join("notes.jsonl");
    fs::write(&unrelated, b"not a Session\n").expect("an unrelated file is written");
    fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o600))
        .expect("unrelated file permissions are restricted");
    let reader = LocalSessionReader::open(directory.path()).expect("reader opens");

    let sessions = reader.discover().expect("listing ignores unrelated files");

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id(), session_id);
}

// 지원되는 v1에 anchor가 없으면 실행 가능한 재개 근거가 없으므로 unavailable이고,
// 미지원 schema는 bounded evidence를 해석할 수 없어 unknown으로 구분해야 합니다.
#[test]
fn continuation_eligibility_distinguishes_missing_anchor_from_unknown_schema() {
    let directory = TestDirectory::new("typed-continuation-eligibility");
    let session_id = session(34);
    {
        let mut writer =
            LocalSessionRepository::open(directory.path(), 32_768).expect("writer opens");
        writer
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("durable")),
            )
            .expect("the supported record is durable");
    }
    let reader = LocalSessionReader::open(directory.path()).expect("reader opens");
    let supported = reader.discover().expect("supported Session is listed");
    assert_eq!(
        supported[0].continuation_eligibility(),
        ContinuationEligibility::Unavailable
    );

    let path = log_path(directory.path(), session_id);
    fs::write(
        &path,
        format!("{{\"schema\":\"yo.session-record/v2\",\"session_id\":\"{session_id}\"}}\n"),
    )
    .expect("an unsupported record replaces the fixture");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("fixture permissions are restricted");

    let unsupported = reader
        .discover()
        .expect("unsupported Session remains inspectable");

    assert_eq!(
        unsupported[0].continuation_eligibility(),
        ContinuationEligibility::Unknown
    );
    assert!(matches!(
        unsupported[0].unavailable_reason(),
        Some(StoredSessionUnavailableReason::UnsupportedSchema { schema })
            if schema == "yo.session-record/v2"
    ));
}

// discovery path만 바꾼 JSON도 payload는 그대로지만 checksum preimage가 달라져야 하므로
// bounded listing 단계에서 신뢰 가능한 summary로 노출되지 않아야 합니다.
#[test]
fn discovery_metadata_is_bound_by_the_envelope_checksum() {
    let directory = TestDirectory::new("discovery-checksum");
    let session_id = session(28);
    {
        let mut writer =
            LocalSessionRepository::open(directory.path(), 32_768).expect("writer opens");
        writer
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("state")),
            )
            .expect("record is durable");
    }
    let path = log_path(directory.path(), session_id);
    let contents = fs::read_to_string(&path).expect("log is readable");
    fs::write(&path, contents.replace("/workspace", "/elsewhere"))
        .expect("discovery metadata is tampered");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("fixture permissions remain restricted");
    let reader = LocalSessionReader::open(directory.path()).expect("reader opens");

    let sessions = reader
        .discover()
        .expect("listing reports per-Session failure");

    assert!(
        sessions[0]
            .unavailable_reason()
            .is_some_and(|reason| reason.to_string().contains("CRC32C"))
    );
}

// append 대상 UUID와 discovery descriptor UUID가 다르면 손상된 envelope를 먼저 쓰고
// 재시작 때 발견하는 대신 writer가 물리 append 전에 거부해야 합니다.
#[test]
fn writer_rejects_a_cross_session_discovery_descriptor() {
    let directory = TestDirectory::new("cross-session-discovery");
    let session_id = session(29);
    let mut writer = LocalSessionRepository::open(directory.path(), 32_768).expect("writer opens");
    let record = DurableRecord::incremental("state")
        .with_discovery(RecordDiscovery::new(crate::fixture_descriptor(session(30))));

    let error = writer
        .append(session_id, record)
        .expect_err("cross-Session discovery is rejected before append");

    assert!(
        error
            .to_string()
            .contains("does not match the append target")
    );
    assert!(!log_path(directory.path(), session_id).exists());
}

// reader open은 없는 저장소를 만들거나 권한을 고치지 않고 그대로 실패해야 read-only
// 경계가 writer 초기화 작업을 암묵적으로 수행하지 않습니다.
#[test]
fn read_port_does_not_create_missing_storage() {
    let directory = TestDirectory::new("read-port-no-create");
    let missing = directory.path().join("missing");

    LocalSessionReader::open(&missing).expect_err("missing storage is not created by a reader");

    assert!(!missing.exists());
}

// 새 writer가 남긴 checksummed v1 record의 payload 한 글자만 바뀌어도 JSON 자체는
// 유효하지만 CRC32C가 달라지므로 replay 전에 complete-line 손상으로 거부해야 합니다.
#[test]
fn rejects_a_checksummed_record_whose_payload_was_changed() {
    let directory = TestDirectory::new("checksum-corruption");
    let session_id = session(21);
    {
        let mut repository =
            LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
        repository
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("alpha")),
            )
            .expect("record is written");
    }
    let path = log_path(directory.path(), session_id);
    let contents = fs::read_to_string(&path).expect("log is readable");
    fs::write(&path, contents.replace("alpha", "omega")).expect("payload is tampered");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("fixture permissions remain restricted");
    let repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");

    let error = repository
        .read_after(session_id, None, 8)
        .expect_err("checksum mismatch is corruption");

    assert!(error.to_string().contains("CRC32C"));
}

// 테스트가 사용하는 레코드 종류도 실제 계약의 두 가지 값과 일치하는지 확인합니다.
#[test]
fn exposes_both_record_kinds_without_storage_details() {
    assert_eq!(
        DurableRecord::incremental("delta").kind(),
        DurableRecordKind::Incremental
    );
    assert_eq!(
        DurableRecord::snapshot("state").kind(),
        DurableRecordKind::Snapshot
    );
}
