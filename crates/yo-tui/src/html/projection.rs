use std::fmt::Write as _;

use super::{
    escape::push_text,
    style::{cell_style, data_attributes, data_color, glyph_style},
};
use crate::surface::{Cell, CellContent, Point, Surface, WIDTH_PROFILE};

/// Canonical HTML/CSS fragment for one completed surface.
pub struct HtmlSurface;

impl HtmlSurface {
    /// CSS required by behavior that cannot be expressed in a style attribute.
    ///
    /// Embed this in a document-level `style` element. It is kept separate so
    /// [`Self::render`] remains a conforming flow-content fragment.
    #[must_use]
    pub const fn stylesheet() -> &'static str {
        "@keyframes yo-surface-blink{50%{visibility:hidden}}"
    }

    /// Render a conforming flow-content fragment for `surface`.
    #[must_use]
    pub fn render(surface: &Surface) -> String {
        let size = surface.size();
        let mut output = String::new();
        writeln!(
            output,
            "<div class=\"yo-surface\" data-width=\"{}\" data-height=\"{}\" data-width-profile=\"{WIDTH_PROFILE}\" style=\"font-family:ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,'Liberation Mono','Courier New',monospace;font-variant-ligatures:none;line-height:1;white-space:pre;\">",
            size.width, size.height
        )
        .expect("writing to String cannot fail");

        for row in 0..size.height {
            let columns = if size.width == 0 {
                "none".to_owned()
            } else {
                format!("repeat({},minmax(0,1ch))", size.width)
            };
            writeln!(
                output,
                "  <div class=\"yo-row\" data-row=\"{row}\" style=\"display:grid;grid-template-columns:{columns};min-height:1em;\">"
            )
            .expect("writing to String cannot fail");
            for column in 0..size.width {
                let cell = surface
                    .cell(Point::new(column, row))
                    .expect("Surface coordinates were derived from its size");
                push_cell(&mut output, cell, column);
            }
            output.push_str("  </div>\n");
        }
        output.push_str("</div>\n");
        output
    }
}

fn push_cell(output: &mut String, cell: &Cell, column: u16) {
    let style = cell.style();
    let foreground = data_color(style.foreground);
    let background = data_color(style.background);
    let attributes = data_attributes(style.attributes);

    match cell.content() {
        CellContent::Blank => {
            writeln!(
                output,
                "    <span class=\"yo-cell yo-blank\" data-column=\"{column}\" data-fg=\"{foreground}\" data-bg=\"{background}\" data-attrs=\"{attributes}\" style=\"{}\"></span>",
                cell_style(style, column, 1)
            )
            .expect("writing to String cannot fail");
        },
        CellContent::Grapheme { text, width } => {
            write!(
                output,
                "    <span class=\"yo-cell yo-grapheme\" data-column=\"{column}\" data-width=\"{}\" data-fg=\"{foreground}\" data-bg=\"{background}\" data-attrs=\"{attributes}\" style=\"{}\">",
                width.get(),
                cell_style(style, column, width.get())
            )
            .expect("writing to String cannot fail");
            write!(
                output,
                "<span class=\"yo-glyph\" style=\"{}\">",
                glyph_style(style)
            )
            .expect("writing to String cannot fail");
            push_text(output, text);
            output.push_str("</span></span>\n");
        },
        CellContent::Continuation { back } => {
            writeln!(
                output,
                "    <span class=\"yo-cell yo-continuation\" data-column=\"{column}\" data-back=\"{}\" data-fg=\"{foreground}\" data-bg=\"{background}\" data-attrs=\"{attributes}\" style=\"{}\" hidden></span>",
                back.get(),
                cell_style(style, column, 1)
            )
            .expect("writing to String cannot fail");
        },
    }
}
