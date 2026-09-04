//! Behavioral tests for the meter primitive.

use super::*;
use crate::{
    GlyphProfile,
    surface::{GraphemeError, cell_width},
};

const TEMPLATE: MeterTemplate<'static> = MeterTemplate::new("{label} {meter} {percent}%");

fn rich_level_spec() -> MeterSpec<'static> {
    MeterSpec::new(
        MeterShape::VerticalLevel,
        MeterGlyphs::for_profile(GlyphProfile::Rich),
        TEMPLATE,
    )
}

// 값 범위를 0~100%로 제한하고 가장 가까운 레벨 글리프로 반올림한다.
#[test]
fn vertical_level_is_clamped_and_rounded() {
    let spec = rich_level_spec();
    assert_eq!(spec.render_glyph(0).unwrap(), "▁");
    assert_eq!(spec.render_glyph(700).unwrap(), "▁");
    assert_eq!(spec.render_glyph(5_000).unwrap(), "▄");
    assert_eq!(spec.render_glyph(10_000).unwrap(), "▇");
    assert_eq!(spec.render_glyph(10_001).unwrap(), "▇");
}

// 가로 막대와 세로 막대가 같은 값에서 채움 비율을 계산하는지 확인한다.
#[test]
fn horizontal_and_vertical_bars_share_the_same_value_semantics() {
    let glyphs = MeterGlyphs::for_profile(GlyphProfile::Rich);
    let horizontal = MeterSpec::new(MeterShape::HorizontalBar { width: 10 }, glyphs, TEMPLATE);
    let vertical = horizontal.with_shape(MeterShape::VerticalBar { height: 4 });

    assert_eq!(horizontal.render_glyph(5_000).unwrap(), "█████░░░░░");
    assert_eq!(horizontal.render_glyph(10_000).unwrap(), "██████████");
    assert_eq!(vertical.render_glyph(5_000).unwrap(), "░\n░\n▇\n▇");
    assert_eq!(vertical.render_glyph(10_000).unwrap(), "▇\n▇\n▇\n▇");
}

// 사용자 정의 level palette가 있어도 기본 filled glyph를 몰래 바꾸지 않고,
// 세로 막대의 inset은 별도 설정을 했을 때만 적용합니다.
#[test]
fn custom_vertical_bar_preserves_or_explicitly_replaces_filled_glyph() {
    let glyphs = MeterGlyphs::new("=", ".", &[".", ":"]);
    let vertical = MeterSpec::raw(MeterShape::VerticalBar { height: 2 }, glyphs);
    assert_eq!(vertical.render_glyph(10_000).unwrap(), "=\n=");

    let inset = MeterSpec::raw(
        MeterShape::VerticalBar { height: 2 },
        glyphs.with_vertical_filled(":"),
    );
    assert_eq!(inset.render_glyph(10_000).unwrap(), ":\n:");
}

// 별칭과 이스케이프한 중괄호를 템플릿에 함께 사용할 수 있어야 한다.
#[test]
fn template_supports_aliases_and_escaped_braces() {
    let spec = rich_level_spec().with_template(MeterTemplate::new("{{{label}}} {bar} = {value}%"));
    assert_eq!(spec.render("7d", 1_800).unwrap(), "{7d} ▂ = 18%");
}

// 출력 문자열을 확장하기 전에 바이트 상한을 검사해 과도한 할당을 막는다.
#[test]
fn template_output_is_bounded_before_expansion_allocates() {
    let oversized_label = "x".repeat(super::MAX_METER_BYTES + 1);
    assert!(matches!(
        MeterTemplate::new("{label}").render(&oversized_label, "█", 0),
        Err(MeterTemplateError::OutputTooLarge { bytes, .. })
            if bytes == oversized_label.len()
    ));

    let oversized_pattern = "x".repeat(super::MAX_METER_BYTES + 1);
    assert!(matches!(
        MeterTemplate::new(oversized_pattern.as_str()).render("label", "█", 0),
        Err(MeterTemplateError::OutputTooLarge { bytes, .. })
            if bytes == oversized_pattern.len()
    ));
}

// 반복된 placeholder까지 포함한 터미널 셀 수 상한을 정확히 적용한다.
#[test]
fn template_cell_budget_covers_labels_and_repeated_placeholders() {
    let oversized_label = "x".repeat(super::MAX_METER_CELLS + 1);
    assert!(matches!(
        MeterTemplate::new("{label}").render(&oversized_label, "█", 0),
        Err(MeterTemplateError::OutputTooLarge { cells, bytes })
            if cells == oversized_label.len() && bytes == oversized_label.len()
    ));

    let repeated_label = "x".repeat(super::MAX_METER_CELLS / 2 + 1);
    assert!(matches!(
        MeterTemplate::new("{label}{label}").render(&repeated_label, "█", 0),
        Err(MeterTemplateError::OutputTooLarge { cells, bytes })
            if cells == repeated_label.len() * 2 && bytes == repeated_label.len() * 2
    ));
}

// 유니코드 폭과 줄바꿈도 템플릿 출력 셀 예산 계산에 반영한다.
#[test]
fn template_cell_budget_uses_terminal_width_for_unicode_and_newlines() {
    let wide_label = "界".repeat(super::MAX_METER_CELLS / 2 + 1);
    assert!(matches!(
        MeterTemplate::new("{label}").render(&wide_label, "█", 0),
        Err(MeterTemplateError::OutputTooLarge { cells, bytes })
            if cells == (super::MAX_METER_CELLS / 2 + 1) * 2
                && bytes == wide_label.len()
    ));

    let combining_label = "e\u{301}".repeat(super::MAX_METER_CELLS);
    let rendered = MeterTemplate::new("{label}")
        .render(&combining_label, "█", 0)
        .unwrap();
    assert_eq!(rendered, combining_label);

    let newline_meter = "\n".repeat(super::MAX_METER_BYTES);
    let rendered = MeterTemplate::new("{meter}")
        .render("label", &newline_meter, 0)
        .unwrap();
    assert_eq!(rendered.len(), super::MAX_METER_BYTES);
}

// 정적 글리프와 수명 짧은 템플릿을 조합하는 빌더 수명을 지원한다.
#[test]
fn builder_accepts_a_borrowed_template_with_static_glyphs() {
    let pattern = String::from("{label}: {meter}");
    let spec = MeterSpec::raw(
        MeterShape::VerticalLevel,
        MeterGlyphs::for_profile(GlyphProfile::Rich),
    )
    .with_template(MeterTemplate::new(pattern.as_str()));

    assert_eq!(spec.render("quota", 5_000).unwrap(), "quota: ▄");
}

// 정적 템플릿과 수명 짧은 사용자 글리프를 조합하는 빌더 수명을 지원한다.
#[test]
fn builder_accepts_borrowed_glyphs_with_a_static_template() {
    let filled = String::from("=");
    let empty = String::from(".");
    let spec = MeterSpec::raw(
        MeterShape::HorizontalBar { width: 2 },
        MeterGlyphs::for_profile(GlyphProfile::Rich),
    )
    .with_glyphs(MeterGlyphs::new(filled.as_str(), empty.as_str(), &[]));

    assert_eq!(spec.render_glyph(5_000).unwrap(), "=.");
}

// 글리프 검증 실패가 원래 surface 계층의 구체적인 원인을 보존하는지 확인한다.
#[test]
fn invalid_glyphs_preserve_surface_validation_errors() {
    let cases = [
        ("", GraphemeError::Empty),
        ("ab", GraphemeError::Multiple),
        ("\u{1b}", GraphemeError::Control),
        ("\u{301}", GraphemeError::ZeroWidth),
    ];

    for (glyph, cause) in cases {
        let spec = MeterSpec::raw(
            MeterShape::HorizontalBar { width: 1 },
            MeterGlyphs::new(glyph, "-", &["."]),
        );
        assert!(matches!(
            spec.render_glyph(0),
            Err(MeterError::InvalidGlyph {
                slot: MeterGlyphSlot::Filled,
                cause: actual,
                ..
            }) if actual == cause
        ));
    }
}

// 잘못된 도형과 셀·바이트 상한을 모두 부분 출력 없이 오류로 닫는다.
#[test]
fn invalid_shapes_and_oversized_outputs_fail_closed() {
    let glyphs = MeterGlyphs::for_profile(GlyphProfile::Ascii);
    assert!(matches!(
        MeterSpec::raw(MeterShape::VerticalLevel, MeterGlyphs::new("#", "-", &[])).render_glyph(0),
        Err(MeterError::EmptyLevels)
    ));
    assert!(matches!(
        MeterSpec::raw(MeterShape::HorizontalBar { width: 0 }, glyphs).render_glyph(0),
        Err(MeterError::ZeroWidth)
    ));
    assert!(matches!(
        MeterSpec::raw(MeterShape::VerticalBar { height: 0 }, glyphs).render_glyph(0),
        Err(MeterError::ZeroHeight)
    ));

    let too_many_levels = vec!["."; super::MAX_METER_LEVELS + 1];
    assert!(matches!(
        MeterSpec::raw(
            MeterShape::VerticalLevel,
            MeterGlyphs::new("#", "-", too_many_levels.as_slice())
        )
        .render_glyph(0),
        Err(MeterError::TooManyLevels { count })
            if count == super::MAX_METER_LEVELS + 1
    ));

    for oversized_cells in [super::MAX_METER_CELLS + 1, usize::MAX] {
        for shape in [
            MeterShape::HorizontalBar {
                width: oversized_cells,
            },
            MeterShape::VerticalBar {
                height: oversized_cells,
            },
        ] {
            assert!(matches!(
                MeterSpec::raw(shape, glyphs).render_glyph(5_000),
                Err(MeterError::RenderTooLarge { cells, .. }) if cells == oversized_cells
            ));
        }
    }

    let oversized_glyph = format!("e{}", "\u{301}".repeat(super::MAX_METER_BYTES));
    let spec = MeterSpec::raw(
        MeterShape::HorizontalBar { width: 1 },
        MeterGlyphs::new(oversized_glyph.as_str(), "-", &["."]),
    );
    assert!(matches!(
        spec.render_glyph(0),
        Err(MeterError::RenderTooLarge { cells: 1, bytes })
            if bytes == oversized_glyph.len()
    ));
}

// ASCII 프로필과 완전한 사용자 정의 글리프·템플릿 조합을 지원한다.
#[test]
fn custom_ascii_glyphs_and_template_are_supported() {
    let spec = MeterSpec::new(
        MeterShape::VerticalLevel,
        MeterGlyphs::for_profile(GlyphProfile::Ascii),
        MeterTemplate::new("{meter}:{percent}"),
    );
    assert_eq!(spec.render("unused", 9_200).unwrap(), "#:92");
    assert_eq!(
        cell_width(&spec.render("unused", 9_200).unwrap()).unwrap(),
        4
    );

    let bars_only = MeterSpec::raw(
        MeterShape::HorizontalBar { width: 4 },
        MeterGlyphs::new("=", ".", &[]),
    );
    assert_eq!(bars_only.render_glyph(5_000).unwrap(), "==..");
}

// 알 수 없는 템플릿과 잘못된 사용자 글리프는 안전한 오류로 거절한다.
#[test]
fn invalid_custom_glyphs_and_templates_fail_closed() {
    let glyphs = MeterGlyphs::new("界", "░", &["▁"]);
    let spec = MeterSpec::new(
        MeterShape::VerticalLevel,
        glyphs,
        MeterTemplate::new("{meter}"),
    );
    assert!(matches!(
        spec.render_glyph(5_000),
        Err(MeterError::GlyphMustBeOneCell {
            slot: MeterGlyphSlot::Filled,
            ..
        })
    ));

    let spec = rich_level_spec().with_template(MeterTemplate::new("{unknown}"));
    assert!(matches!(
        spec.render("label", 5_000),
        Err(MeterError::Template(
            MeterTemplateError::UnknownPlaceholder(_)
        ))
    ));

    let spec = rich_level_spec().with_template(MeterTemplate::new("{meter"));
    assert!(matches!(
        spec.render("label", 5_000),
        Err(MeterError::Template(
            MeterTemplateError::UnterminatedPlaceholder
        ))
    ));

    let template = MeterTemplate::new("{meter}");
    assert!(matches!(
        template.render("label", "\u{1b}[31m", 5_000),
        Err(MeterTemplateError::ControlCharacter('\u{1b}'))
    ));
}

// 백분율 표시가 제공자 값의 필요한 정밀도만 유지하는지 확인한다.
#[test]
fn percent_formatting_preserves_provider_precision() {
    assert_eq!(format_percent(0), "0");
    assert_eq!(format_percent(1_800), "18");
    assert_eq!(format_percent(1_801), "18.01");
    assert_eq!(format_percent(1_810), "18.1");
    assert_eq!(format_percent(10_001), "100");
}
