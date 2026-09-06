use yo_tui::surface::cell_width;

use super::*;

fn width(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value).unwrap()
}

// 여러 모델 목록은 쉼표 경계에서만 다음 줄로 넘겨 충분한 폭이 있는데도 긴 모델 ID를
// 중간에서 자르지 않으며, 각 모델을 정확히 한 번씩 그대로 보여 줍니다.
#[test]
fn model_list_wraps_between_models_without_splitting_identifiers() {
    let models = [
        "anthropic/claude-opus-4",
        "deepseek/deepseek-r1",
        "mistralai/mistral-large",
    ];

    let lines = wrap_list(&models, 30).unwrap();

    assert_eq!(
        lines,
        [
            "anthropic/claude-opus-4,",
            "deepseek/deepseek-r1,",
            "mistralai/mistral-large",
        ]
    );
}

// Model ID 자체는 inline 폭에 맞지만 뒤 구분자까지 맞지 않으면 쉼표를 별도 줄로
// 떼거나 ID를 자르지 않고 명확한 bullet 항목으로 전환합니다.
#[test]
fn model_list_uses_bullets_when_an_inline_separator_would_not_fit() {
    let mut output = String::new();

    push_model_list_field(
        &mut output,
        "Models",
        &["abcde", "f"],
        23,
        PresentationStyle::Plain,
    )
    .unwrap();

    assert_eq!(output, "  Models\n  • abcde\n  • f\n");
}

// structured parameter의 연속 공백도 의미 있는 문자열 bytes일 수 있으므로 wrapping 전후
// 조각을 이어 붙이면 원문과 같고 split_whitespace식 정규화가 일어나지 않습니다.
#[test]
fn wrapping_preserves_exact_whitespace_in_profile_values() {
    let original = r#"{"note":"a  b"}"#;
    let wrapped = wrap(original, 7).unwrap();

    assert_eq!(wrapped.concat(), original);
    assert!(wrapped.iter().all(|line| cell_width(line).unwrap() <= 7));
}

// 독립 default-ignorable grapheme은 폭 0이라 보이지 않는 내용을 confirmation에
// 몰래 섞을 수 있으므로 일반 문자처럼 허용하지 않고 terminal-safe 경계에서 거절합니다.
#[test]
fn wrapping_rejects_an_isolated_zero_width_grapheme() {
    assert!(matches!(
        wrap("\u{200b}", 80),
        Err(PresentationError::UnsafeText(GraphemeError::ZeroWidth))
    ));
}

// terminal stdout snapshot은 nonzero winsize를 사용하고 zero 또는 non-terminal이면 80열로
// 돌아가며, ANSI 선택은 같은 snapshot의 terminal 여부와 NO_COLOR 입력에만 따릅니다.
#[test]
fn success_snapshot_uses_terminal_width_with_zero_and_redirected_fallbacks() {
    use nix::pty::openpty;
    use rustix::termios::{Winsize, tcsetwinsize};

    let pty = openpty(None, None).unwrap();
    tcsetwinsize(
        &pty.slave,
        Winsize {
            ws_row: 24,
            ws_col: 37,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )
    .unwrap();
    assert_eq!(
        SuccessPresentation::for_output(&pty.slave, true, false),
        SuccessPresentation {
            width: width(37),
            style: PresentationStyle::Ansi,
        }
    );
    assert_eq!(
        SuccessPresentation::for_output(&pty.slave, true, true).style,
        PresentationStyle::Plain
    );

    tcsetwinsize(
        &pty.slave,
        Winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )
    .unwrap();
    assert_eq!(
        SuccessPresentation::for_output(&pty.slave, true, false).width,
        default_width()
    );

    let redirected = std::fs::File::open("/dev/null").unwrap();
    assert_eq!(
        SuccessPresentation::for_output(&redirected, false, false),
        SuccessPresentation {
            width: default_width(),
            style: PresentationStyle::Plain,
        }
    );
}
