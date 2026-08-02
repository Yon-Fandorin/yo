use super::super::{Grapheme, GraphemeError, WIDTH_PROFILE, cell_width};

// 고정한 Unicode 폭 프로필의 식별자가 외부 adapter에도 관찰 가능한지 확인한다.
#[test]
fn exposes_the_pinned_width_profile() {
    assert_eq!(WIDTH_PROFILE, "yo-unicode-17.0-narrow/v1");
    assert_eq!(unicode_segmentation::UNICODE_VERSION, (17, 0, 0));
    assert_eq!(emojis::UNICODE_VERSION.major(), 17);
    assert_eq!(emojis::UNICODE_VERSION.minor(), 0);
}

// ASCII, 한글, 결합 문자와 Unicode 17 신규 한자의 폭을 같은 규칙으로 계산한다.
#[test]
fn resolves_non_emoji_grapheme_widths() {
    let cases = [("A", 1), ("가", 2), ("e\u{301}", 1), ("\u{323b0}", 2)];

    for (text, width) in cases {
        assert_eq!(Grapheme::try_from(text).unwrap().width().get(), width);
    }
}

// CLI 표와 Surface가 ASCII, 한글, 결합문자의 합성 폭을 서로 다르게 계산하지 않도록
// 여러 grapheme으로 이루어진 문자열도 같은 고정 프로필로 합산합니다.
#[test]
fn measures_complete_text_with_the_surface_width_profile() {
    assert_eq!(cell_width("A가e\u{301}").unwrap(), 4);
    assert_eq!(cell_width("\u{301}").unwrap(), 0);
}

// ZWJ emoji, 국기, 기본 emoji 표현과 표준 VS16 표현을 폭 2로 확정한다.
#[test]
fn resolves_emoji_grapheme_widths() {
    let cases = ["👩‍💻", "🇰🇷", "😀", "♥️"];

    for text in cases {
        assert_eq!(Grapheme::try_from(text).unwrap().width().get(), 2);
    }
}

// 표준 VS15는 emoji 기본 표현을 text 표현으로 바꾸되 원문 UTF-8을 보존한다.
#[test]
fn standardized_vs15_uses_non_emoji_width() {
    let grapheme = Grapheme::try_from("♥︎\u{301}").unwrap();

    assert_eq!(grapheme.as_str(), "♥︎\u{301}");
    assert_eq!(grapheme.width().get(), 1);
}

// cluster 내부의 text-default 문자가 가진 표준 VS16 sequence도 폭 2로 처리한다.
#[test]
fn standardized_vs16_inside_a_cluster_uses_emoji_width() {
    assert_eq!(Grapheme::try_from("♥️\u{301}").unwrap().width().get(), 2);
    assert_eq!(Grapheme::try_from("#️").unwrap().width().get(), 2);
    assert_eq!(Grapheme::try_from("#︎").unwrap().width().get(), 1);
}

// 비표준 variation selector는 폭에 영향을 주지 않고 0 기여 문자로만 처리한다.
#[test]
fn nonstandard_variation_selector_is_neutral() {
    assert_eq!(Grapheme::try_from("A\u{fe0f}").unwrap().width().get(), 1);
}

// 빈 값, 여러 cluster, control, 폭이 0인 cluster를 mutation 전에 거절한다.
#[test]
fn rejects_invalid_grapheme_inputs() {
    assert_eq!(Grapheme::try_from(""), Err(GraphemeError::Empty));
    assert_eq!(Grapheme::try_from("AB"), Err(GraphemeError::Multiple));
    assert_eq!(Grapheme::try_from("\n"), Err(GraphemeError::Control));
    assert_eq!(Grapheme::try_from("\u{301}"), Err(GraphemeError::ZeroWidth));
}
