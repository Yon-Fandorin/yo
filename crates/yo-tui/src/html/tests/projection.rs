use super::write;
use crate::{
    html::HtmlSurface,
    surface::{Point, Size, Style, Surface},
};

// 작은 completed Surface를 byte-for-byte 고정된 canonical fragment로 투영한다.
#[test]
fn projection_has_an_exact_canonical_shape() {
    let mut surface = Surface::new(Size::new(2, 1)).unwrap();
    write(&mut surface, Point::new(1, 0), "A", Style::default());

    assert_eq!(
        HtmlSurface::render(&surface),
        concat!(
            "<div class=\"yo-surface\" data-width=\"2\" data-height=\"1\" data-width-profile=\"yo-unicode-17.0-narrow/v1\" style=\"font-family:ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,'Liberation Mono','Courier New',monospace;font-variant-ligatures:none;line-height:1;white-space:pre;\">\n",
            "  <div class=\"yo-row\" data-row=\"0\" style=\"display:grid;grid-template-columns:repeat(2,minmax(0,1ch));min-height:1em;\">\n",
            "    <span class=\"yo-cell yo-blank\" data-column=\"0\" data-fg=\"default\" data-bg=\"default\" data-attrs=\"\" style=\"grid-column:1/span 1;min-width:0;overflow:hidden;background-color:var(--yo-default-background,#000000);\"></span>\n",
            "    <span class=\"yo-cell yo-grapheme\" data-column=\"1\" data-width=\"1\" data-fg=\"default\" data-bg=\"default\" data-attrs=\"\" style=\"grid-column:2/span 1;min-width:0;overflow:hidden;background-color:var(--yo-default-background,#000000);\"><span class=\"yo-glyph\" style=\"display:block;inline-size:100%;block-size:100%;overflow:hidden;color:var(--yo-default-foreground,#d0d0d0);\">A</span></span>\n",
            "  </div>\n",
            "</div>\n",
        )
    );
}

// root metadata는 물리 크기와 선택된 Unicode width profile을 명시한다.
#[test]
fn root_records_surface_size_and_width_profile() {
    let surface = Surface::new(Size::new(2, 1)).unwrap();
    let fragment = HtmlSurface::render(&surface);

    assert!(fragment.starts_with(
        "<div class=\"yo-surface\" data-width=\"2\" data-height=\"1\" data-width-profile=\"yo-unicode-17.0-narrow/v1\" style=\"font-family:"
    ));
    assert!(fragment.ends_with("</div>\n"));
}

// wide grapheme은 leader 한 곳에 원문과 폭을 두고 continuation의 back-reference도 보존한다.
#[test]
fn wide_grapheme_preserves_leader_and_continuation_occupancy() {
    let mut surface = Surface::new(Size::new(4, 1)).unwrap();
    write(&mut surface, Point::new(1, 0), "가", Style::default());

    let fragment = HtmlSurface::render(&surface);

    assert!(fragment.contains("class=\"yo-cell yo-grapheme\" data-column=\"1\" data-width=\"2\""));
    assert!(fragment.contains("grid-column:2/span 2;"));
    assert!(fragment.contains(">가</span>"));
    assert!(
        fragment.contains("class=\"yo-cell yo-continuation\" data-column=\"2\" data-back=\"1\"")
    );
    assert!(fragment.contains("hidden></span>"));
}

// zero-width·zero-height Surface도 viewer chrome 없는 유효한 root fragment로 남는다.
#[test]
fn empty_dimensions_have_a_stable_fragment() {
    let surface = Surface::new(Size::new(0, 0)).unwrap();

    assert_eq!(
        HtmlSurface::render(&surface),
        concat!(
            "<div class=\"yo-surface\" data-width=\"0\" data-height=\"0\" data-width-profile=\"yo-unicode-17.0-narrow/v1\" style=\"font-family:ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,'Liberation Mono','Courier New',monospace;font-variant-ligatures:none;line-height:1;white-space:pre;\">\n",
            "</div>\n",
        )
    );
}

// width가 0이고 row가 존재할 때도 유효한 empty CSS grid를 사용한다.
#[test]
fn zero_width_rows_use_an_empty_grid_definition() {
    let surface = Surface::new(Size::new(0, 1)).unwrap();
    let fragment = HtmlSurface::render(&surface);

    assert!(fragment.contains(
        "<div class=\"yo-row\" data-row=\"0\" style=\"display:grid;grid-template-columns:none;min-height:1em;\">"
    ));
    assert!(!fragment.contains("repeat(0,"));
}

// document metadata CSS는 flow-content fragment와 분리된 canonical 계약으로 제공한다.
#[test]
fn stylesheet_is_separate_from_the_flow_content_fragment() {
    let surface = Surface::new(Size::new(1, 1)).unwrap();
    let fragment = HtmlSurface::render(&surface);

    assert_eq!(
        HtmlSurface::stylesheet(),
        "@keyframes yo-surface-blink{50%{visibility:hidden}}"
    );
    assert!(!fragment.contains("<style"));
}
