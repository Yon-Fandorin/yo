use super::super::{
    Attributes, CellContent, Color, Grapheme, Point, Rect, Size, Style, Surface, WriteOutcome,
};

fn style(index: u8) -> Style {
    Style::new(
        Color::Indexed(index),
        Color::Indexed(index + 1),
        Attributes::BOLD,
    )
}

fn full_view(surface: &mut Surface) -> super::super::SurfaceView<'_> {
    let size = surface.size();
    surface.view(Rect::new(Point::new(0, 0), size)).unwrap()
}

// 새 Surface가 terminal-default Style의 명시적인 Blank 셀로 채워지는지 확인한다.
#[test]
fn new_surface_contains_default_blanks() {
    let surface = Surface::new(Size::new(2, 1)).unwrap();

    for x in 0..2 {
        let cell = surface.cell(Point::new(x, 0)).unwrap();
        assert_eq!(cell.content(), &CellContent::Blank);
        assert_eq!(cell.style(), Style::default());
    }
}

// clear가 view 안의 모든 셀을 요청된 resolved Style의 Blank로 바꾸는지 확인한다.
#[test]
fn clear_replaces_the_complete_view_with_styled_blanks() {
    let mut surface = Surface::new(Size::new(3, 1)).unwrap();
    let cleared = style(8);
    let mut view = full_view(&mut surface);
    assert_eq!(
        view.write(
            Point::new(0, 0),
            Grapheme::try_from("가").unwrap(),
            style(1)
        ),
        WriteOutcome::Written
    );

    assert_eq!(view.clear(cleared), WriteOutcome::Written);
    for x in 0..3 {
        let cell = view.cell(Point::new(x, 0)).unwrap();
        assert_eq!(cell.content(), &CellContent::Blank);
        assert_eq!(cell.style(), cleared);
    }
}

// 폭 2 grapheme을 leader 하나와 올바른 back-reference continuation으로 기록한다.
#[test]
fn wide_write_creates_one_complete_footprint() {
    let mut surface = Surface::new(Size::new(4, 1)).unwrap();
    let incoming = style(2);
    let mut view = full_view(&mut surface);

    assert_eq!(
        view.write(
            Point::new(1, 0),
            Grapheme::try_from("가").unwrap(),
            incoming
        ),
        WriteOutcome::Written
    );
    assert!(matches!(
        view.cell(Point::new(1, 0)).unwrap().content(),
        CellContent::Grapheme { text, width }
            if text.as_ref() == "가" && width.get() == 2
    ));
    assert!(matches!(
        view.cell(Point::new(2, 0)).unwrap().content(),
        CellContent::Continuation { back } if back.get() == 1
    ));
    assert_eq!(view.cell(Point::new(2, 0)).unwrap().style(), incoming);
}

// 남은 view 폭에 grapheme 전체가 들어가지 않으면 기존 상태를 전혀 바꾸지 않는다.
#[test]
fn clipped_write_is_atomic() {
    let mut surface = Surface::new(Size::new(3, 1)).unwrap();
    let before = surface.clone();
    {
        let mut view = full_view(&mut surface);
        assert_eq!(
            view.write(
                Point::new(2, 0),
                Grapheme::try_from("가").unwrap(),
                style(3)
            ),
            WriteOutcome::Clipped
        );
    }
    assert_eq!(surface, before);
}

// 좁은 문자가 넓은 leader를 덮으면 이전 footprint 전체를 incoming style Blank로 정리한다.
#[test]
fn narrower_write_cleans_the_old_wide_footprint() {
    let mut surface = Surface::new(Size::new(4, 1)).unwrap();
    let incoming = style(4);
    let mut view = full_view(&mut surface);
    assert_eq!(
        view.write(
            Point::new(1, 0),
            Grapheme::try_from("가").unwrap(),
            style(1)
        ),
        WriteOutcome::Written
    );

    assert_eq!(
        view.write(Point::new(1, 0), Grapheme::try_from("A").unwrap(), incoming),
        WriteOutcome::Written
    );
    assert_eq!(
        view.cell(Point::new(2, 0)).unwrap().content(),
        &CellContent::Blank
    );
    assert_eq!(view.cell(Point::new(2, 0)).unwrap().style(), incoming);
}

// continuation에서 시작한 쓰기도 기존 leader와 continuation을 함께 정리한다.
#[test]
fn continuation_start_write_closes_over_the_owner() {
    let mut surface = Surface::new(Size::new(4, 1)).unwrap();
    let incoming = style(5);
    let mut view = full_view(&mut surface);
    assert_eq!(
        view.write(
            Point::new(1, 0),
            Grapheme::try_from("가").unwrap(),
            style(1)
        ),
        WriteOutcome::Written
    );

    assert_eq!(
        view.write(Point::new(2, 0), Grapheme::try_from("A").unwrap(), incoming),
        WriteOutcome::Written
    );
    assert_eq!(
        view.cell(Point::new(1, 0)).unwrap().content(),
        &CellContent::Blank
    );
    assert_eq!(view.cell(Point::new(1, 0)).unwrap().style(), incoming);
}

// 새 wide footprint가 서로 다른 기존 grapheme 둘과 겹치면 둘의 전체 영역을 함께 닫는다.
#[test]
fn wide_write_closes_over_every_intersecting_footprint() {
    let mut surface = Surface::new(Size::new(5, 1)).unwrap();
    let incoming = style(9);
    let mut view = full_view(&mut surface);

    // 첫 grapheme의 continuation과 둘째 grapheme의 leader가 새 footprint에 동시에 겹친다.
    for (x, text) in [(0, "가"), (2, "나")] {
        assert_eq!(
            view.write(
                Point::new(x, 0),
                Grapheme::try_from(text).unwrap(),
                style(1)
            ),
            WriteOutcome::Written
        );
    }
    assert_eq!(
        view.write(
            Point::new(1, 0),
            Grapheme::try_from("한").unwrap(),
            incoming
        ),
        WriteOutcome::Written
    );

    assert_eq!(
        view.cell(Point::new(0, 0)).unwrap().content(),
        &CellContent::Blank
    );
    assert_eq!(
        view.cell(Point::new(3, 0)).unwrap().content(),
        &CellContent::Blank
    );
    assert_eq!(view.cell(Point::new(0, 0)).unwrap().style(), incoming);
    assert_eq!(view.cell(Point::new(3, 0)).unwrap().style(), incoming);
}

// 작은 view가 바깥에서 시작한 grapheme과 교차하면 component 경계를 훼손하지 않는다.
#[test]
fn crossing_old_footprint_clips_without_mutation() {
    let mut surface = Surface::new(Size::new(4, 1)).unwrap();
    {
        let mut view = full_view(&mut surface);
        assert_eq!(
            view.write(
                Point::new(0, 0),
                Grapheme::try_from("가").unwrap(),
                style(1)
            ),
            WriteOutcome::Written
        );
    }
    let before = surface.clone();
    {
        let mut component = surface
            .view(Rect::new(Point::new(1, 0), Size::new(2, 1)))
            .unwrap();
        assert_eq!(
            component.write(Point::new(0, 0), Grapheme::try_from("A").unwrap(), style(6)),
            WriteOutcome::Clipped
        );
    }
    assert_eq!(surface, before);
}

// clear도 view를 가로지르는 기존 footprint가 있으면 원자적으로 실패한다.
#[test]
fn clear_preserves_crossing_footprints() {
    let mut surface = Surface::new(Size::new(4, 1)).unwrap();
    {
        let mut view = full_view(&mut surface);
        assert_eq!(
            view.write(
                Point::new(0, 0),
                Grapheme::try_from("가").unwrap(),
                style(1)
            ),
            WriteOutcome::Written
        );
    }
    let before = surface.clone();
    {
        let mut component = surface
            .view(Rect::new(Point::new(1, 0), Size::new(2, 1)))
            .unwrap();
        assert_eq!(component.clear(style(7)), WriteOutcome::Clipped);
    }
    assert_eq!(surface, before);
}

// subview 좌표는 부모 view 원점에 상대적이며 쓰기는 계산된 절대 영역에만 반영된다.
#[test]
fn nested_view_uses_parent_relative_coordinates() {
    let mut surface = Surface::new(Size::new(6, 3)).unwrap();
    {
        let mut parent = surface
            .view(Rect::new(Point::new(2, 1), Size::new(3, 2)))
            .unwrap();
        let mut child = parent
            .subview(Rect::new(Point::new(1, 0), Size::new(2, 1)))
            .unwrap();

        assert_eq!(
            child.write(Point::new(0, 0), Grapheme::try_from("A").unwrap(), style(3)),
            WriteOutcome::Written
        );
    }

    assert!(matches!(
        surface.cell(Point::new(3, 1)).unwrap().content(),
        CellContent::Grapheme { text, .. } if text.as_ref() == "A"
    ));
    assert_eq!(
        surface.cell(Point::new(2, 1)).unwrap().content(),
        &CellContent::Blank
    );
}

// subview가 부모 범위를 넘으면 Surface를 빌려주지 않는다.
#[test]
fn nested_view_rejects_invalid_relative_geometry() {
    let mut surface = Surface::new(Size::new(4, 1)).unwrap();
    let mut parent = surface
        .view(Rect::new(Point::new(2, 0), Size::new(1, 1)))
        .unwrap();

    assert!(matches!(
        parent.subview(Rect::new(Point::new(1, 0), Size::new(1, 1))),
        Err(super::super::GeometryError::OutOfBounds)
    ));
}

// 링크는 넓은 grapheme 전체에 붙고 부분 view 실패는 원자적이며 덮어쓰기·지우기로 제거된다.
#[test]
fn hyperlink_ownership_follows_the_complete_grapheme_footprint() {
    use super::super::Hyperlink;
    let link = Hyperlink::new("https://example.com/docs").unwrap();
    let mut surface = Surface::new(Size::new(3, 1)).unwrap();
    assert_eq!(
        full_view(&mut surface).write_linked(
            Point::new(0, 0),
            Grapheme::try_from("한").unwrap(),
            style(1),
            Some(link.clone())
        ),
        WriteOutcome::Written
    );
    assert_eq!(
        surface.cell(Point::new(0, 0)).unwrap().hyperlink(),
        Some(&link)
    );
    assert_eq!(
        surface.cell(Point::new(1, 0)).unwrap().hyperlink(),
        Some(&link)
    );
    let original = surface.clone();
    let mut partial = surface
        .view(Rect::new(Point::new(1, 0), Size::new(2, 1)))
        .unwrap();
    assert_eq!(
        partial.write(Point::new(0, 0), Grapheme::try_from("x").unwrap(), style(2)),
        WriteOutcome::Clipped
    );
    assert_eq!(surface, original);
    assert_eq!(
        full_view(&mut surface).write(Point::new(1, 0), Grapheme::try_from("x").unwrap(), style(2)),
        WriteOutcome::Written
    );
    assert!(!surface.has_hyperlinks());
    full_view(&mut surface).write_linked(
        Point::new(0, 0),
        Grapheme::try_from("한").unwrap(),
        style(1),
        Some(link),
    );
    assert_eq!(
        full_view(&mut surface).clear(style(0)),
        WriteOutcome::Written
    );
    assert!(!surface.has_hyperlinks());
}

// 외부 실행 스킴·제어 문자와 주소 길이 첫 초과는 링크화하지 않고 HTTP(S)만 받아들인다.
#[test]
fn hyperlink_destinations_are_bounded_web_urls_without_controls() {
    use super::super::Hyperlink;
    for destination in [
        "https://example.com/한글?q=a#b",
        "http://localhost:8080/a",
        "HTTPS://example.com/a",
    ] {
        assert_eq!(
            Hyperlink::new(destination).unwrap().destination(),
            destination
        );
    }
    for destination in [
        "",
        "relative/path",
        "file:///tmp/file",
        "javascript:alert(1)",
        "mailto:a@example.com",
        "https://",
        "https://example.com/\u{1b}]52;c;secret\u{7}",
        "https://example.com/\u{9c}",
        "https://example.com/\nspoof",
    ] {
        assert!(Hyperlink::new(destination).is_none(), "{destination:?}");
    }
    let prefix = "https://example.com/";
    let exact = format!("{prefix}{}", "x".repeat(8192 - prefix.len()));
    assert!(Hyperlink::new(&exact).is_some());
    assert!(Hyperlink::new(&(exact + "x")).is_none());
}

// 파일 목적지는 호스트 전용 절대 경로 생성자로만 만들며 URL 메타 문자는 경로 데이터로 인코딩한다.
// 상대·부모 경로·제어 문자를 거부하고 인코딩 후 8 KiB의 정확한 한도와 첫 초과를 검사한다.
#[test]
fn trusted_file_hyperlinks_keep_web_constructor_closed() {
    use std::path::Path;

    use url::Url;

    use super::super::Hyperlink;
    let path = Path::new("/tmp/yo 한글#?.rs");
    let link = Hyperlink::from_file_path(path).unwrap();
    let parsed = Url::parse(link.destination()).unwrap();
    assert_eq!(parsed.scheme(), "file");
    assert_eq!(parsed.to_file_path().unwrap(), path);
    assert!(parsed.query().is_none());
    assert!(parsed.fragment().is_none());
    assert!(Hyperlink::new(link.destination()).is_none());
    for value in [
        "relative.rs",
        "file:///tmp/a",
        "/tmp/../a",
        "/tmp/line\nname",
        "/tmp/\u{1b}name",
    ] {
        assert!(
            Hyperlink::from_file_path(Path::new(value)).is_none(),
            "{value:?}"
        );
    }
    let exact = format!("/{}", " ".repeat(2728));
    assert_eq!(
        Hyperlink::from_file_path(Path::new(&exact))
            .unwrap()
            .destination()
            .len(),
        8 * 1024
    );
    assert!(Hyperlink::from_file_path(Path::new(&(exact + "x"))).is_none());
}
