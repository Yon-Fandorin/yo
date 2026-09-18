use super::SecretInput;

// 비밀 입력은 UTF-8 바이트를 그대로 보존하되 Debug 출력에는 값을 노출하지 않는지 확인한다.
#[test]
fn secret_input_preserves_exact_utf8_without_debug_disclosure() {
    let value = "한글\nline\0🙂";
    let input = SecretInput::new(value).unwrap();

    assert_eq!(input.expose(), value);
    assert_eq!(format!("{input:?}"), "SecretInput([REDACTED])");
    assert!(!format!("{input:?}").contains(value));
}

// 비밀 입력의 64KiB 제한을 ASCII와 다중 바이트 문자 모두에서 정확히 적용하는지 확인한다.
#[test]
fn secret_input_enforces_the_exact_utf8_byte_boundary() {
    assert!(SecretInput::new("a".repeat(SecretInput::MAX_BYTES)).is_ok());
    assert!(SecretInput::new("a".repeat(SecretInput::MAX_BYTES + 1)).is_err());

    let multibyte = "가".repeat(SecretInput::MAX_BYTES / 3);
    assert!(SecretInput::new(multibyte.clone()).is_ok());
    assert!(SecretInput::new(format!("{multibyte}가")).is_err());
}
