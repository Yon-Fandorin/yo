use super::*;

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

    let report = check::failed_authority_report(checkpoint::AuthorityFailure::from_source(
        "0123456789abcdef",
        failure,
    ));
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
