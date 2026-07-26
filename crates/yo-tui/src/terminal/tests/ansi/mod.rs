mod content;
mod errors;
mod style;

use crate::{
    surface::{FrameDiff, Grapheme, Point, Rect, Style, Surface, WriteOutcome},
    terminal::{AnsiEncoder, TerminalOps},
};

fn write(surface: &mut Surface, point: Point, text: &str, style: Style) {
    let size = surface.size();
    let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
    assert_eq!(
        view.write(point, Grapheme::try_from(text).unwrap(), style),
        WriteOutcome::Written
    );
}

fn encode(previous: &Surface, current: &Surface) -> Vec<u8> {
    let diff = FrameDiff::between(previous, current);
    let operations = TerminalOps::from_diff(&diff);
    let mut encoder = AnsiEncoder::new(Vec::new());
    encoder.encode(&operations).unwrap();
    encoder.into_inner()
}
