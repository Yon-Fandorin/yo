use super::PreparedTranscript;

impl PreparedTranscript {
    pub(crate) fn into_plain_text(self) -> Option<String> {
        let mut glyphs = self.layout.glyphs.into_iter();
        let first = glyphs.next()?;
        let mut output = String::new();
        let mut row = 0_u16;
        let mut column = 0_u16;

        append_gap(
            &mut output,
            &mut row,
            &mut column,
            first.point.x,
            first.point.y,
        );
        column += first.grapheme.width().get();
        output.push_str(first.grapheme.as_str());

        for positioned in glyphs {
            append_gap(
                &mut output,
                &mut row,
                &mut column,
                positioned.point.x,
                positioned.point.y,
            );
            column += positioned.grapheme.width().get();
            output.push_str(positioned.grapheme.as_str());
        }
        output.push('\n');
        Some(output)
    }
}

fn append_gap(output: &mut String, row: &mut u16, column: &mut u16, x: u16, y: u16) {
    while *row < y {
        output.push('\n');
        *row += 1;
        *column = 0;
    }
    debug_assert!(
        x >= *column,
        "prepared transcript glyphs must be ordered and non-overlapping"
    );
    output.extend(std::iter::repeat_n(' ', usize::from(x - *column)));
    *column = x;
}
