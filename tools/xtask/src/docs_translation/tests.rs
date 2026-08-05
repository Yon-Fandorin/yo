use std::{fs, path::Path, process::Command};

use super::{
    CANONICAL_ROOT, KOREAN_ROOT, MANIFEST, accept, publish_reviewed_hash, sha256_hex,
    storage::RepositoryFiles, update_manifest, validated_page,
};
use crate::test_support::TestRepository;

fn fixture(label: &str) -> TestRepository {
    let repository = TestRepository::new(label);
    repository.write("docs/src/README.md", "# Canonical\n");
    repository.write("docs/ko/src/README.md", "# 한국어\n");
    repository.write(
        "docs/ko/source.sha256",
        "0000000000000000000000000000000000000000000000000000000000000000  README.md\n\
         1111111111111111111111111111111111111111111111111111111111111111  other.md\n",
    );
    repository
}

// 번역 검수가 끝난 한 페이지를 승인하면 그 원문의 SHA-256 한 행만
// 교체하고, 다른 페이지의 기록과 manifest 줄바꿈은 그대로 보존한다.
#[test]
fn accepts_exactly_one_reviewed_page_without_rewriting_neighbors() {
    let repository = fixture("docs-accept-one");

    accept(&repository.path, Path::new("README.md")).unwrap();

    let expected = format!(
        "{}  README.md\n\
         1111111111111111111111111111111111111111111111111111111111111111  other.md\n",
        sha256_hex(b"# Canonical\n")
    );
    assert_eq!(
        fs::read_to_string(repository.path.join("docs/ko/source.sha256")).unwrap(),
        expected
    );
}

// 등록되지 않은 페이지는 새 행을 추측해 추가하지 않고 실패하여 manifest의
// 닫힌 페이지 집합을 우회하지 못하게 한다.
#[test]
fn rejects_a_page_missing_from_the_manifest_without_mutation() {
    let repository = fixture("docs-missing-entry");
    repository.write("docs/src/new.md", "# New\n");
    repository.write("docs/ko/src/new.md", "# 새 문서\n");
    let before = fs::read(repository.path.join("docs/ko/source.sha256")).unwrap();

    let error = accept(&repository.path, Path::new("new.md")).unwrap_err();

    assert!(error.contains("has no entry"));
    assert_eq!(
        fs::read(repository.path.join("docs/ko/source.sha256")).unwrap(),
        before
    );
}

// 상위 디렉터리나 절대 경로를 입력해도 docs/src 밖의 파일을 해시하지
// 못하도록 CLI 입력 경계에서 경로 탈출을 거부한다.
#[test]
fn page_paths_are_relative_markdown_paths_without_traversal() {
    assert!(validated_page(Path::new("architecture/overview.md")).is_ok());
    assert!(validated_page(Path::new("../README.md")).is_err());
    assert!(validated_page(Path::new("/tmp/README.md")).is_err());
    assert!(validated_page(Path::new("README.txt")).is_err());
    assert!(validated_page(Path::new("unsupported\\name.md")).is_err());
}

// canonical 또는 한국어 Projection 경로에 symbolic link가 끼어 있으면
// 링크 대상이 저장소 안이어도 승인하지 않아 실제 검토 대상을 바꾸지 못한다.
#[test]
fn rejects_symlinked_projection_paths_without_touching_the_manifest() {
    let repository = fixture("docs-symlink");
    let manifest = repository.path.join("docs/ko/source.sha256");
    let before = fs::read(&manifest).unwrap();
    fs::remove_file(repository.path.join("docs/ko/src/README.md")).unwrap();
    std::os::unix::fs::symlink(
        repository.path.join("docs/src/README.md"),
        repository.path.join("docs/ko/src/README.md"),
    )
    .unwrap();

    let error = accept(&repository.path, Path::new("README.md")).unwrap_err();

    assert!(error.contains("symbolic links"));
    assert_eq!(fs::read(manifest).unwrap(), before);
}

// 중복 등록된 페이지는 어느 행이 권위인지 임의로 고르지 않고 실패하여
// manifest 모순을 사람이 먼저 해결하게 한다.
#[test]
fn duplicate_manifest_entries_fail_closed() {
    let manifest = b"0000000000000000000000000000000000000000000000000000000000000000  README.md\n\
                     1111111111111111111111111111111111111111111111111111111111111111  README.md\n";

    let error = update_manifest(manifest, "README.md", &"2".repeat(64)).unwrap_err();

    assert!(error.contains("duplicate entries"));
}

// 선택하지 않은 페이지라도 중복되거나 저장소 밖을 가리키면 전체 manifest를
// 모순으로 거부하여 일부 정상 행만 고쳐 잘못된 목록을 승인하지 않는다.
#[test]
fn every_manifest_entry_must_be_unique_safe_and_normalized() {
    let duplicate_other = b"0000000000000000000000000000000000000000000000000000000000000000  README.md\n\
                            1111111111111111111111111111111111111111111111111111111111111111  other.md\n\
                            2222222222222222222222222222222222222222222222222222222222222222  other.md\n";
    let escaping = b"0000000000000000000000000000000000000000000000000000000000000000  README.md\n\
                     1111111111111111111111111111111111111111111111111111111111111111  ../outside.md\n";

    assert!(
        update_manifest(duplicate_other, "README.md", &"3".repeat(64))
            .unwrap_err()
            .contains("duplicate entries")
    );
    assert!(update_manifest(escaping, "README.md", &"3".repeat(64)).is_err());
}

// CRLF와 마지막 newline 부재도 manifest의 기존 bytes이므로 선택한 digest
// 외에는 그대로 보존해 불필요한 전체 파일 재작성을 만들지 않는다.
#[test]
fn selected_digest_replacement_preserves_line_endings_and_final_newline_state() {
    let manifest =
        b"0000000000000000000000000000000000000000000000000000000000000000  README.md\r\n\
                     1111111111111111111111111111111111111111111111111111111111111111  other.md";

    let updated = update_manifest(manifest, "README.md", &"a".repeat(64)).unwrap();

    assert_eq!(
        updated,
        b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  README.md\r\n\
          1111111111111111111111111111111111111111111111111111111111111111  other.md"
    );
}

// 파일 크기는 metadata만 믿지 않고 실제 읽기에도 상한을 적용하여, 변경 중인
// 비정상 입력이 승인 명령의 메모리를 제한 없이 사용하지 못하게 한다.
#[test]
fn bounded_reader_rejects_content_beyond_its_limit() {
    let repository = fixture("docs-bounded-read");
    let files = RepositoryFiles::open(&repository.path).unwrap();

    let error = files
        .capture(Path::new("docs/src/README.md"), 4)
        .err()
        .unwrap();

    assert!(error.contains("exceeds the 4 byte limit"));
}

// FIFO를 nonblocking으로 연 뒤 regular-file 검사에서 즉시 거부하여 writer가
// 나타날 때까지 번역 승인 명령이 멈추는 회귀를 막는다.
#[test]
fn fifo_input_is_rejected_without_waiting_for_a_writer() {
    let repository = fixture("docs-fifo");
    let canonical = repository.path.join("docs/src/README.md");
    fs::remove_file(&canonical).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(&canonical)
            .status()
            .unwrap()
            .success()
    );
    let files = RepositoryFiles::open(&repository.path).unwrap();

    let error = files
        .capture(Path::new("docs/src/README.md"), 32)
        .err()
        .unwrap();

    assert!(error.contains("regular files"));
}

// 같은 저장소에서 두 승인 명령이 겹치면 두 번째 명령을 실패시켜 서로의
// manifest 교체를 덮지 않도록 협력적 writer 경계를 직렬화한다.
#[test]
fn cooperating_accept_commands_do_not_overlap() {
    let repository = fixture("docs-lock");
    let _first = RepositoryFiles::open(&repository.path).unwrap();

    let error = RepositoryFiles::open(&repository.path).err().unwrap();

    assert!(error.contains("another cooperating repository mutation"));
}

// 임시 manifest를 준비하는 동안 외부 편집이 관찰되면 최종 재검증에서
// 교체를 중단하고 외부 bytes를 보존하며 생성한 임시 파일도 제거한다.
#[test]
fn detected_external_manifest_edit_is_not_overwritten() {
    let repository = fixture("docs-concurrent-edit");
    let files = RepositoryFiles::open(&repository.path).unwrap();
    let captured = files.capture(Path::new(MANIFEST), 1024).unwrap();
    let manifest_path = repository.path.join(MANIFEST);
    let external = b"2222222222222222222222222222222222222222222222222222222222222222  README.md\n\
                     1111111111111111111111111111111111111111111111111111111111111111  other.md\n";
    let updated = update_manifest(captured.bytes(), "README.md", &"a".repeat(64)).unwrap();

    let error = captured
        .atomic_replace_guarded(&updated, || {
            fs::write(&manifest_path, external).map_err(|error| error.to_string())
        })
        .unwrap_err();

    assert!(error.contains("changed before publication"));
    assert_eq!(fs::read(&manifest_path).unwrap(), external);
    let temporary_remains = fs::read_dir(manifest_path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".source.sha256.tmp-")
        });
    assert!(!temporary_remains);
}

// capture 뒤 canonical 원문이나 한국어 Projection이 바뀌면 실제로 검토한
// 입력 조합이 아니므로 최종 guard가 hash 기록을 게시하지 않는다.
#[test]
fn canonical_and_korean_edits_both_stop_hash_publication() {
    for (label, changed_path, replacement) in [
        ("canonical", "docs/src/README.md", "# Changed canonical\n"),
        ("korean", "docs/ko/src/README.md", "# 바뀐 번역\n"),
    ] {
        let repository = fixture(&format!("docs-input-change-{label}"));
        let files = RepositoryFiles::open(&repository.path).unwrap();
        let canonical = files
            .capture(
                &Path::new(CANONICAL_ROOT).join("README.md"),
                8 * 1024 * 1024,
            )
            .unwrap();
        let korean = files
            .capture(&Path::new(KOREAN_ROOT).join("README.md"), 8 * 1024 * 1024)
            .unwrap();
        let manifest = files.capture(Path::new(MANIFEST), 1024 * 1024).unwrap();
        let before = manifest.bytes().to_vec();
        let updated = update_manifest(&before, "README.md", &"a".repeat(64)).unwrap();
        fs::write(repository.path.join(changed_path), replacement).unwrap();

        let error = publish_reviewed_hash(&canonical, &korean, &manifest, &updated).unwrap_err();

        assert!(error.contains("changed before publication"));
        assert_eq!(fs::read(repository.path.join(MANIFEST)).unwrap(), before);
    }
}

// final guard 전에 canonical 파일이 사라지면 어느 README인지 모호한 basename이
// 아니라 전체 저장소 상대 경로를 진단하여 수정할 입력을 바로 찾게 한다.
#[test]
fn final_guard_diagnostic_identifies_the_changed_input_path() {
    let repository = fixture("docs-input-disappeared");
    let files = RepositoryFiles::open(&repository.path).unwrap();
    let canonical = files
        .capture(
            &Path::new(CANONICAL_ROOT).join("README.md"),
            8 * 1024 * 1024,
        )
        .unwrap();
    let korean = files
        .capture(&Path::new(KOREAN_ROOT).join("README.md"), 8 * 1024 * 1024)
        .unwrap();
    let manifest = files.capture(Path::new(MANIFEST), 1024 * 1024).unwrap();
    let updated = update_manifest(manifest.bytes(), "README.md", &"a".repeat(64)).unwrap();
    fs::remove_file(repository.path.join("docs/src/README.md")).unwrap();

    let error = publish_reviewed_hash(&canonical, &korean, &manifest, &updated).unwrap_err();

    assert!(error.contains("docs/src/README.md"));
}
