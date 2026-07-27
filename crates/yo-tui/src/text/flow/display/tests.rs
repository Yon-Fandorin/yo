use super::control_notation;

// C0 전체 범위는 ^@부터 ^_까지 손실 없이 ASCII caret 표기로 바뀐다.
#[test]
fn every_c0_control_has_caret_notation() {
    for code in 0_u32..=0x1f {
        let control = char::from_u32(code).unwrap();
        let expected = format!("^{}", char::from_u32(code + 0x40).unwrap());

        assert_eq!(
            control_notation(&control.to_string()).as_deref(),
            Some(expected.as_str())
        );
    }
}

// DEL은 C0 연속 범위 밖에서도 관례적인 ^? 표기를 사용한다.
#[test]
fn delete_has_caret_notation() {
    assert_eq!(control_notation("\u{7f}").as_deref(), Some("^?"));
}

// C1 전체 범위는 모호한 glyph 대신 정확한 4자리 Unicode 코드 표기를 사용한다.
#[test]
fn every_c1_control_has_unicode_notation() {
    for code in 0x80_u32..=0x9f {
        let control = char::from_u32(code).unwrap();
        let expected = format!("\\u{{{code:04X}}}");

        assert_eq!(
            control_notation(&control.to_string()).as_deref(),
            Some(expected.as_str())
        );
    }
}
