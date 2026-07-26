mod escaping;
mod projection;
mod style;

use crate::surface::{Grapheme, Point, Rect, Style, Surface, WriteOutcome};

fn write(surface: &mut Surface, point: Point, text: &str, style: Style) {
    let size = surface.size();
    let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
    assert_eq!(
        view.write(point, Grapheme::try_from(text).unwrap(), style),
        WriteOutcome::Written
    );
}
