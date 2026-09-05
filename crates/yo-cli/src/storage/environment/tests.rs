use std::{ffi::OsString, path::PathBuf};

use super::{
    DEFAULT_CAPACITY_BYTES, StorageConfigError, capacity_bytes_from, platform_state_root_from,
    repository_root_from,
};

// 명시적인 repository override는 OS 기본 위치보다 먼저 선택되어 test와 운영자가
// 같은 단일 writer root를 의도적으로 지정할 수 있어야 한다.
#[test]
fn explicit_repository_root_has_priority() {
    let state_root = PathBuf::from("/tmp/xdg/yo");
    let root = repository_root_from(Some(OsString::from("/tmp/yo-explicit")), &state_root).unwrap();

    assert_eq!(root, PathBuf::from("/tmp/yo-explicit"));
}

// capacity 환경값이 없으면 제품 기본 1 GiB를 사용하고, 숫자가 아닌 값은 조용히
// fallback하지 않아 사용자가 잘못된 저장 한도를 즉시 알 수 있어야 한다.
#[test]
fn capacity_uses_the_default_and_rejects_invalid_input() {
    assert_eq!(capacity_bytes_from(None).unwrap(), DEFAULT_CAPACITY_BYTES);
    assert!(capacity_bytes_from(Some(OsString::from("1GiB"))).is_err());
    assert_eq!(
        capacity_bytes_from(Some(OsString::from("4096"))).unwrap(),
        4096
    );
}

// Host identity 위치가 현재 작업 디렉터리에 따라 달라지면 같은 사용자도 다른
// Host로 보이므로, 플랫폼별 상태 환경변수의 상대 경로를 typed 오류로 거부합니다.
#[test]
fn platform_state_root_rejects_relative_environment_paths() {
    #[cfg(target_os = "macos")]
    let result = platform_state_root_from(None, Some(OsString::from("relative-home")));
    #[cfg(not(target_os = "macos"))]
    let result = platform_state_root_from(
        Some(OsString::from("relative-state")),
        Some(OsString::from("/tmp/home")),
    );

    let error = result.expect_err("a relative platform state root must be rejected");
    #[cfg(target_os = "macos")]
    assert!(matches!(
        error,
        StorageConfigError::InvalidEnvironment {
            name: "HOME",
            ref reason,
        } if reason == "path is not absolute"
    ));
    #[cfg(not(target_os = "macos"))]
    assert!(matches!(
        error,
        StorageConfigError::InvalidEnvironment {
            name: "XDG_STATE_HOME",
            ref reason,
        } if reason == "path is not absolute"
    ));
}

#[cfg(not(target_os = "macos"))]
// Linux에서는 XDG state 위치가 HOME fallback보다 우선해 다른 XDG-aware CLI와
// 동일한 사용자 상태 디렉터리 규칙을 지킨다.
#[test]
fn linux_prefers_xdg_state_home() {
    let root = platform_state_root_from(
        Some(OsString::from("/tmp/xdg")),
        Some(OsString::from("/tmp/home")),
    )
    .unwrap();

    assert_eq!(root, PathBuf::from("/tmp/xdg/yo"));
}

#[cfg(not(target_os = "macos"))]
// XDG_STATE_HOME이 없으면 HOME의 표준 state 경로를 선택해야 신규 환경에서도
// Session과 Host identity가 문서화된 같은 기본 위치를 공유합니다.
#[test]
fn linux_unset_xdg_state_home_falls_back_to_home() {
    let root = platform_state_root_from(None, Some(OsString::from("/tmp/home"))).unwrap();

    assert_eq!(root, PathBuf::from("/tmp/home/.local/state/yo"));
}

#[cfg(not(target_os = "macos"))]
// XDG 규약에서 빈 XDG_STATE_HOME은 unset과 같으므로, 유효한 HOME이 있으면 빈
// 경로 오류 대신 unset과 정확히 같은 HOME state 경로를 선택해야 합니다.
#[test]
fn linux_empty_xdg_state_home_falls_back_to_home() {
    let root =
        platform_state_root_from(Some(OsString::new()), Some(OsString::from("/tmp/home"))).unwrap();

    assert_eq!(root, PathBuf::from("/tmp/home/.local/state/yo"));
}

#[cfg(not(target_os = "macos"))]
// 빈 XDG_STATE_HOME이 fallback을 허용해도 빈 HOME까지 유효해지는 것은 아니므로,
// 실제 경로를 만들기 전에 HOME의 typed 빈 경로 오류를 그대로 반환해야 합니다.
#[test]
fn linux_empty_xdg_state_home_does_not_accept_empty_home() {
    let error = platform_state_root_from(Some(OsString::new()), Some(OsString::new()))
        .expect_err("an empty HOME must remain invalid after XDG fallback");

    assert!(matches!(
        error,
        StorageConfigError::InvalidEnvironment {
            name: "HOME",
            ref reason,
        } if reason == "path is empty"
    ));
}

#[cfg(not(target_os = "macos"))]
// 빈 XDG_STATE_HOME이 unset으로 정규화돼도 HOME 자체가 없으면 기본 위치를 추측하지
// 않고, 경로 선택 전에 HOME의 typed 미설정 오류를 반환해야 합니다.
#[test]
fn linux_empty_xdg_state_home_does_not_accept_unset_home() {
    let error = platform_state_root_from(Some(OsString::new()), None)
        .expect_err("an unset HOME must remain invalid after XDG fallback");

    assert!(matches!(
        error,
        StorageConfigError::InvalidEnvironment {
            name: "HOME",
            ref reason,
        } if reason == "value is not set"
    ));
}

#[cfg(not(target_os = "macos"))]
// 빈 XDG_STATE_HOME 뒤의 상대 HOME은 현재 작업 디렉터리에 의존하는 state root가
// 되지 않도록, HOME 소유의 typed 절대 경로 오류로 실패해야 합니다.
#[test]
fn linux_empty_xdg_state_home_does_not_accept_relative_home() {
    let error =
        platform_state_root_from(Some(OsString::new()), Some(OsString::from("relative-home")))
            .expect_err("a relative HOME must remain invalid after XDG fallback");

    assert!(matches!(
        error,
        StorageConfigError::InvalidEnvironment {
            name: "HOME",
            ref reason,
        } if reason == "path is not absolute"
    ));
}

#[cfg(all(unix, not(target_os = "macos")))]
// 비어 있지 않은 절대 raw-byte XDG_STATE_HOME은 OS 경로로 유효하므로 UTF-8 여부와
// 무관하게 HOME fallback보다 우선하고, 원래 경로 bytes를 보존해야 합니다.
#[test]
fn linux_absolute_non_utf8_xdg_state_home_remains_selected() {
    use std::os::unix::ffi::OsStringExt;

    let mut state = b"/tmp/xdg-".to_vec();
    state.push(0xff);
    let expected = PathBuf::from(OsString::from_vec(state.clone())).join("yo");
    let root = platform_state_root_from(
        Some(OsString::from_vec(state)),
        Some(OsString::from("/tmp/home")),
    )
    .unwrap();

    assert_eq!(root, expected);
    assert_ne!(root, PathBuf::from("/tmp/home/.local/state/yo"));
}

#[cfg(target_os = "macos")]
// macOS에서는 별도 override가 없으면 사용자 Library의 Application Support 아래를
// 사용해 Session 파일이 일반 문서나 project 디렉터리에 섞이지 않게 한다.
#[test]
fn macos_uses_application_support() {
    let root = platform_state_root_from(None, Some(OsString::from("/tmp/home"))).unwrap();

    assert_eq!(
        root,
        PathBuf::from("/tmp/home/Library/Application Support/yo")
    );
}

#[cfg(target_os = "macos")]
// macOS는 XDG_STATE_HOME의 값과 무관하게 HOME 아래 Application Support를 쓰는 기존
// 선택 규칙을 유지해야 Linux용 empty-as-unset 변경이 플랫폼 경계를 넘지 않습니다.
#[test]
fn macos_continues_to_ignore_xdg_state_home() {
    let root = platform_state_root_from(
        Some(OsString::from("/tmp/xdg")),
        Some(OsString::from("/tmp/home")),
    )
    .unwrap();

    assert_eq!(
        root,
        PathBuf::from("/tmp/home/Library/Application Support/yo")
    );
}
