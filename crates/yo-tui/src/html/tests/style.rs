use super::write;
use crate::{
    html::HtmlSurface,
    surface::{Attributes, Color, Point, Size, Style, Surface},
};

// RGB와 indexed/default 색상 identity를 data와 CSS에 함께 보존한다.
#[test]
fn colors_have_deterministic_data_and_css_representations() {
    let mut surface = Surface::new(Size::new(1, 1)).unwrap();
    let style = Style::new(
        Color::Rgb {
            red: 1,
            green: 2,
            blue: 3,
        },
        Color::Indexed(42),
        Attributes::empty(),
    );
    write(&mut surface, Point::new(0, 0), "A", style);

    let fragment = HtmlSurface::render(&surface);

    assert!(fragment.contains("data-fg=\"rgb-1-2-3\" data-bg=\"indexed-42\""));
    assert!(fragment.contains("background-color:var(--yo-color-42,#00d787);"));
    assert!(fragment.contains("color:rgb(1 2 3);"));
}

// xterm indexed palette의 각 구간 경계는 CSS 변수 없이도 같은 색으로 재현된다.
#[test]
fn indexed_colors_have_stable_xterm_fallbacks() {
    let cases = [
        (0, "#000000"),
        (15, "#ffffff"),
        (16, "#000000"),
        (231, "#ffffff"),
        (232, "#080808"),
        (255, "#eeeeee"),
    ];

    for (index, fallback) in cases {
        let mut surface = Surface::new(Size::new(1, 1)).unwrap();
        write(
            &mut surface,
            Point::new(0, 0),
            "A",
            Style::new(Color::Indexed(index), Color::Default, Attributes::empty()),
        );

        assert!(
            HtmlSurface::render(&surface)
                .contains(&format!("color:var(--yo-color-{index},{fallback});")),
            "indexed color {index} should fall back to {fallback}"
        );
    }
}

// resolved attribute를 안정적인 이름과 대응 CSS로 투영한다.
#[test]
fn attributes_keep_semantics_and_stable_order() {
    let mut surface = Surface::new(Size::new(1, 1)).unwrap();
    let attributes = Attributes::BOLD
        .union(Attributes::DIM)
        .union(Attributes::ITALIC)
        .union(Attributes::UNDERLINE)
        .union(Attributes::BLINK)
        .union(Attributes::HIDDEN)
        .union(Attributes::STRIKETHROUGH);
    write(
        &mut surface,
        Point::new(0, 0),
        "A",
        Style::new(Color::Default, Color::Default, attributes),
    );

    let fragment = HtmlSurface::render(&surface);

    assert!(
        fragment.contains("data-attrs=\"bold dim italic underline blink hidden strikethrough\"")
    );
    assert!(fragment.contains(
        "color:color-mix(in srgb,var(--yo-default-foreground,#d0d0d0) 50%,var(--yo-default-background,#000000));"
    ));
    assert!(fragment.contains("font-weight:700;font-style:italic;"));
    assert!(fragment.contains("text-decoration-line:underline line-through;"));
    assert!(fragment.contains("animation:yo-surface-blink 1s step-end infinite;"));
    assert!(fragment.contains("visibility:hidden;"));
    assert!(!fragment.contains("background-color:var(--yo-default-background,#000000);animation:"));
}

// reverse는 raw resolved color identity를 바꾸지 않고 CSS 표시 순서만 뒤집는다.
#[test]
fn reverse_swaps_css_colors_but_preserves_resolved_data() {
    let mut surface = Surface::new(Size::new(1, 1)).unwrap();
    let style = Style::new(Color::Indexed(1), Color::Indexed(2), Attributes::REVERSE);
    write(&mut surface, Point::new(0, 0), "A", style);

    let fragment = HtmlSurface::render(&surface);

    assert!(
        fragment.contains("data-fg=\"indexed-1\" data-bg=\"indexed-2\" data-attrs=\"reverse\"")
    );
    assert!(fragment.contains("background-color:var(--yo-color-1,#800000);"));
    assert!(fragment.contains("color:var(--yo-color-2,#008000);"));
}
