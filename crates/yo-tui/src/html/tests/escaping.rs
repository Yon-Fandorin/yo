use super::write;
use crate::{
    html::HtmlSurface,
    surface::{Point, Size, Style, Surface},
};

// grapheme text의 HTML 특수 문자를 escape해 fragment 구조와 text 의미를 함께 보존한다.
#[test]
fn grapheme_text_is_html_escaped() {
    for (text, escaped) in [
        ("&", "&amp;"),
        ("<", "&lt;"),
        (">", "&gt;"),
        ("\"", "&quot;"),
        ("'", "&#39;"),
    ] {
        let mut surface = Surface::new(Size::new(1, 1)).unwrap();
        write(&mut surface, Point::new(0, 0), text, Style::default());

        let fragment = HtmlSurface::render(&surface);

        assert!(fragment.contains(&format!(">{escaped}</span>")));
    }
}
