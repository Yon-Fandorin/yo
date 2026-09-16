use std::{error, fmt};

use yo_tui::surface::GraphemeError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PresentationError {
    UnsafeText(GraphemeError),
    GraphemeExceedsWidth { grapheme_width: usize, width: usize },
    InvalidPlan(&'static str),
}

impl fmt::Display for PresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsafeText(error) => write!(
                formatter,
                "connection preview is not terminal-safe: {error}"
            ),
            Self::GraphemeExceedsWidth {
                grapheme_width,
                width,
            } => write!(
                formatter,
                "a {grapheme_width}-cell preview grapheme cannot fit the {width}-cell terminal width"
            ),
            Self::InvalidPlan(message) => formatter.write_str(message),
        }
    }
}

impl error::Error for PresentationError {}

impl From<GraphemeError> for PresentationError {
    fn from(error: GraphemeError) -> Self {
        Self::UnsafeText(error)
    }
}
