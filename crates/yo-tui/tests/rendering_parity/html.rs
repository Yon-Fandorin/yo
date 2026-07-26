use yo_tui::html::HtmlSurface;

use super::support::fixture_surface;

// 동일 Surface의 HTML fragment를 byte-for-byte golden으로 고정한다.
#[test]
fn html_matches_the_shared_surface_fixture() {
    assert_eq!(
        HtmlSurface::render(&fixture_surface()),
        include_str!("../fixtures/rendering-parity/expected.html")
    );
}

// HTML behavior CSS도 fragment와 함께 재현 가능한 golden으로 고정한다.
#[test]
fn html_stylesheet_matches_the_shared_surface_fixture() {
    assert_eq!(
        HtmlSurface::stylesheet(),
        include_str!("../fixtures/rendering-parity/expected.css").trim_end()
    );
}
