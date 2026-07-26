use yo_tui::{
    surface::{
        Attributes, Color, FrameDiff, Grapheme, Point, Rect, Size, Style, Surface, WriteOutcome,
    },
    terminal::TerminalOps,
};

pub(super) const SIZE: Size = Size::new(6, 2);

pub(super) fn fixture_surface() -> Surface {
    let mut surface = Surface::new(SIZE).unwrap();
    write(
        &mut surface,
        Point::new(0, 0),
        "A",
        Style::new(
            Color::Rgb {
                red: 1,
                green: 2,
                blue: 3,
            },
            Color::Indexed(42),
            Attributes::BOLD.union(Attributes::UNDERLINE),
        ),
    );
    write(
        &mut surface,
        Point::new(1, 0),
        "가",
        Style::new(Color::Indexed(15), Color::Indexed(4), Attributes::empty()),
    );
    surface
        .view(Rect::new(Point::new(3, 0), Size::new(1, 1)))
        .unwrap()
        .clear(Style::new(
            Color::Default,
            Color::Indexed(1),
            Attributes::empty(),
        ));
    write(
        &mut surface,
        Point::new(4, 0),
        "👩‍💻",
        Style::new(
            Color::Rgb {
                red: 200,
                green: 150,
                blue: 100,
            },
            Color::Default,
            Attributes::DIM.union(Attributes::ITALIC),
        ),
    );
    write(
        &mut surface,
        Point::new(0, 1),
        "<",
        Style::new(
            Color::Indexed(1),
            Color::Indexed(2),
            Attributes::REVERSE.union(Attributes::STRIKETHROUGH),
        ),
    );
    write(
        &mut surface,
        Point::new(1, 1),
        "&",
        Style::new(
            Color::Default,
            Color::Default,
            Attributes::BLINK.union(Attributes::HIDDEN),
        ),
    );
    surface
}

pub(super) fn fixture_ops(surface: &Surface) -> TerminalOps<'_> {
    let previous = Surface::new(SIZE).unwrap();
    let diff = FrameDiff::between(&previous, surface);
    TerminalOps::from_diff(&diff)
}

fn write(surface: &mut Surface, point: Point, text: &str, style: Style) {
    let mut view = surface.view(Rect::new(Point::new(0, 0), SIZE)).unwrap();
    assert_eq!(
        view.write(point, Grapheme::try_from(text).unwrap(), style),
        WriteOutcome::Written
    );
}
