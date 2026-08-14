use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

struct Fixture {
    root: PathBuf,
    path: PathBuf,
}

impl Fixture {
    fn new(contents: &[u8], mode: u32) -> Self {
        let root = super::super::super::canonical_test_temp_dir().join(format!(
            "yo-credential-input-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("credential");
        fs::write(&path, contents).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        Self { root, path }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn read(path: &Path) -> Result<ApiCredential, AppError> {
    read_credential_file_with(path, rustix::process::geteuid().as_raw(), || Ok(()))
}

// 0400 파일의 마지막 LF 하나만 제거하고 앞뒤 공백을 포함한 나머지 credential bytes는
// 그대로 보존하여 mounted secret과 agent가 만든 파일을 같은 규칙으로 읽습니다.
#[test]
fn accepts_owner_read_only_file_and_removes_one_lf() {
    let fixture = Fixture::new(b"  secret value  \n", 0o400);

    let credential = read(&fixture.path).unwrap();

    assert_eq!(credential.expose_secret(), "  secret value  ");
    assert_eq!(fs::read(&fixture.path).unwrap(), b"  secret value  \n");
}

// 0600 파일의 CRLF 한 쌍만 제거하며 그 앞의 LF는 credential 내부 control로 남아
// ApiCredential 검증에서 거절되므로 여러 줄 secret을 조용히 바꾸지 않습니다.
#[test]
fn accepts_one_crlf_but_rejects_an_additional_line_break() {
    let accepted = Fixture::new(b"secret\r\n", 0o600);
    assert_eq!(read(&accepted.path).unwrap().expose_secret(), "secret");

    let rejected = Fixture::new(b"first\nsecond\r\n", 0o600);
    assert!(
        read(&rejected.path)
            .unwrap_err()
            .to_string()
            .contains("control")
    );
}

// Terminal LF/CRLF 외의 control은 보존한 채 ApiCredential에서 거절하고, file bytes가
// UTF-8이 아니거나 line ending 제거 뒤 비면 다른 문자열로 대체하지 않습니다.
#[test]
fn rejects_invalid_utf8_lone_cr_and_empty_value() {
    let invalid_utf8 = Fixture::new(&[0xff, b'\n'], 0o600);
    assert!(
        read(&invalid_utf8.path)
            .unwrap_err()
            .to_string()
            .contains("valid UTF-8")
    );

    let lone_cr = Fixture::new(b"secret\r", 0o600);
    assert!(
        read(&lone_cr.path)
            .unwrap_err()
            .to_string()
            .contains("control")
    );

    let empty = Fixture::new(b"\n", 0o600);
    assert!(
        read(&empty.path)
            .unwrap_err()
            .to_string()
            .contains("1 to 16384 bytes")
    );
}

// final symlink과 directory는 no-follow regular-file 경계를 통과하지 못하고 target bytes를
// credential로 읽지 않아 경로 교체나 특수 파일을 비대화형 secret source로 만들지 않습니다.
#[test]
fn rejects_symlink_and_non_regular_file() {
    let fixture = Fixture::new(b"secret", 0o600);
    let link = fixture.root.join("credential-link");
    symlink(&fixture.path, &link).unwrap();

    assert!(read(&link).is_err());
    assert!(read(&fixture.root).is_err());
}

// 허용 목록이 정확히 0400/0600이므로 group/world bit뿐 아니라 0200만 있는 파일도
// 거절하고, 잘못된 expected uid를 주입하면 실제 파일 내용을 진단에 노출하지 않습니다.
#[test]
fn rejects_wrong_mode_and_owner_without_exposing_secret() {
    for mode in [0o440, 0o640, 0o644, 0o700, 0o4600] {
        let fixture = Fixture::new(b"sentinel-secret", mode);
        let error = read(&fixture.path).unwrap_err().to_string();
        assert!(error.contains("0400 or 0600"), "mode {mode:o}: {error}");
        assert!(!error.contains("sentinel-secret"));
    }

    let fixture = Fixture::new(b"owner-sentinel", 0o600);
    let actual = rustix::process::geteuid().as_raw();
    let error = read_credential_file_with(&fixture.path, actual.wrapping_add(1), || Ok(()))
        .unwrap_err()
        .to_string();
    assert!(error.contains("current user"));
    assert!(!error.contains("owner-sentinel"));
}

// 초기 크기와 EOF 뒤 실제 capture 크기는 16,386 bytes 이하이어야 하고 terminal CRLF를
// 뺀 payload만 ApiCredential의 16,384-byte 상한에 정확히 도달할 수 있습니다.
#[test]
fn enforces_file_and_credential_size_boundaries() {
    let mut maximum = vec![b'x'; MAX_CREDENTIAL_BYTES];
    maximum.extend_from_slice(b"\r\n");
    let accepted = Fixture::new(&maximum, 0o600);
    assert_eq!(
        read(&accepted.path).unwrap().expose_secret().len(),
        MAX_CREDENTIAL_BYTES
    );

    let oversized_file = Fixture::new(&vec![b'x'; MAX_FILE_BYTES + 1], 0o600);
    assert!(
        read(&oversized_file.path)
            .unwrap_err()
            .to_string()
            .contains("16,386")
    );

    let oversized_secret = Fixture::new(&vec![b'x'; MAX_CREDENTIAL_BYTES + 1], 0o600);
    assert!(
        read(&oversized_secret.path)
            .unwrap_err()
            .to_string()
            .contains("16,384")
    );
}

// handle read 뒤 같은 pathname을 더 긴 내용으로 바꾸는 seam은 before/after metadata 또는
// capture 길이 불일치로 실패하여 불안정한 일부 bytes를 credential로 승인하지 않습니다.
#[test]
fn rejects_a_file_changed_during_capture() {
    let fixture = Fixture::new(b"old-secret", 0o600);

    let error =
        read_credential_file_with(&fixture.path, rustix::process::geteuid().as_raw(), || {
            fs::write(&fixture.path, b"new-secret-with-different-size")
                .map_err(|error| AppError::single("mutating the test credential", error))
        })
        .unwrap_err()
        .to_string();

    assert!(error.contains("changed while"));
    assert!(!error.contains("old-secret"));
    assert!(!error.contains("new-secret"));
}
