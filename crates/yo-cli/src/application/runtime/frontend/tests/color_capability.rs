use yo_tui::ColorCapability;

use super::super::classify_terminal_color_capability;

// 명시적인 truecolor 표시는 24-bit ramp 사용을 허용하고 대소문자 차이는 의미를 바꾸지 않는다.
#[test]
fn explicit_color_term_selects_true_color() {
    assert_eq!(
        classify_terminal_color_capability(Some("TRUECOLOR"), Some("xterm-256color"), false),
        ColorCapability::TrueColor
    );
    assert_eq!(
        classify_terminal_color_capability(Some("24bit"), None, false),
        ColorCapability::TrueColor
    );
}

// 256-color TERM만 확인되면 RGB를 과장하지 않고 제한 색상 fallback을 선택한다.
#[test]
fn term_256color_selects_the_limited_fallback() {
    assert_eq!(
        classify_terminal_color_capability(None, Some("screen-256color"), false),
        ColorCapability::Limited
    );
}

// NO_COLOR 또는 아무 증거가 없는 환경은 RGB를 내보내지 않는 Unknown 경계로 닫는다.
#[test]
fn missing_or_suppressed_color_evidence_stays_unknown() {
    assert_eq!(
        classify_terminal_color_capability(Some("truecolor"), None, true),
        ColorCapability::Unknown
    );
    assert_eq!(
        classify_terminal_color_capability(None, None, false),
        ColorCapability::Unknown
    );
}
