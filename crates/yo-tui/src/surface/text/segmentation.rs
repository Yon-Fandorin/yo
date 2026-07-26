use std::{error::Error, fmt, num::NonZeroU16};

use unicode_segmentation::UnicodeSegmentation;

use super::width::display_width;

/// One validated extended grapheme cluster with a resolved physical width.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Grapheme {
    text: Box<str>,
    width: NonZeroU16,
}

impl Grapheme {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub const fn width(&self) -> NonZeroU16 {
        self.width
    }

    pub(crate) fn into_parts(self) -> (Box<str>, NonZeroU16) {
        (self.text, self.width)
    }
}

impl TryFrom<&str> for Grapheme {
    type Error = GraphemeError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut clusters = value.graphemes(true);
        let Some(cluster) = clusters.next() else {
            return Err(GraphemeError::Empty);
        };
        if cluster.len() != value.len() || clusters.next().is_some() {
            return Err(GraphemeError::Multiple);
        }
        if value.chars().any(char::is_control) {
            return Err(GraphemeError::Control);
        }

        let width = NonZeroU16::new(display_width(value)?).ok_or(GraphemeError::ZeroWidth)?;
        Ok(Self {
            text: value.into(),
            width,
        })
    }
}

/// Why text cannot become one physical grapheme.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphemeError {
    Empty,
    Multiple,
    Control,
    ZeroWidth,
}

impl fmt::Display for GraphemeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("grapheme is empty"),
            Self::Multiple => formatter.write_str("input contains multiple grapheme clusters"),
            Self::Control => formatter.write_str("grapheme contains a control character"),
            Self::ZeroWidth => formatter.write_str("grapheme has zero display width"),
        }
    }
}

impl Error for GraphemeError {}
