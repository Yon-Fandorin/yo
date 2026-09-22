#[cfg(target_os = "linux")]
use std::os::unix::fs::symlink;
use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{DraftFile, MAX_DRAFT_BYTES, parse_argv, read_utf8_bounded};

fn test_path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the system clock is after the Unix epoch")
        .as_nanos();
    env::temp_dir().join(format!("yo-editor-test-{}-{name}-{nonce}", process::id()))
}

// 따옴표는 argv 구분만 바꾸고 셸 확장 문법은 문자 그대로 유지한다.
#[test]
fn parser_preserves_literal_shell_syntax() {
    assert_eq!(
        parse_argv("code --wait 'draft file.md' \"$HOME\" $(echo nope)").unwrap(),
        [
            "code",
            "--wait",
            "draft file.md",
            "$HOME",
            "$(echo",
            "nope)"
        ]
    );
    assert_eq!(parse_argv("program '' \"\"").unwrap(), ["program", "", ""]);
}

// 닫히지 않은 따옴표나 이스케이프는 자식을 만들기 전에 거부한다.
#[test]
fn parser_rejects_incomplete_quoting() {
    assert!(parse_argv("editor '").is_err());
    assert!(parse_argv("editor \\").is_err());
}

// 초안 파일의 권한을 제한하고 사용 뒤 소유한 디렉터리까지 제거한다.
#[test]
fn draft_file_is_private_and_cleanup_is_complete() {
    let draft = DraftFile::create("seed").unwrap();
    let path = draft.path().to_owned();
    let directory = path.parent().unwrap().to_owned();
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    draft.cleanup().unwrap();
    assert!(!path.exists());
    assert!(!directory.exists());
}

// 한도를 넘는 첫 바이트와 잘못된 UTF-8은 TUI에 전달하지 않는다.
#[test]
fn read_is_bounded_before_utf8_decoding() {
    let path = test_path("bounded");
    fs::write(&path, vec![b'a'; MAX_DRAFT_BYTES + 1]).unwrap();
    let error = read_utf8_bounded(&path).unwrap_err().to_string();
    assert!(error.contains("byte limit"));
    fs::write(&path, [0xff]).unwrap();
    assert!(read_utf8_bounded(&path).is_err());
    let _ = fs::remove_file(path);
}

#[cfg(target_os = "linux")]
// 비공개 디렉터리 안에서도 교체된 심볼릭 링크는 읽지 않는다.
#[test]
fn read_rejects_symlink_replacement() {
    let path = test_path("symlink");
    let target = test_path("target");
    fs::write(&target, "secret").unwrap();
    symlink(&target, &path).unwrap();
    assert!(read_utf8_bounded(&path).is_err());
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(target);
}
