use std::{error::Error, fmt};

/// A failure to start, operate, or restore a live TUI session.
#[derive(Debug)]
pub struct RunError {
    context: &'static str,
    detail: String,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl RunError {
    pub(super) fn new(context: &'static str, detail: impl Into<String>) -> Self {
        Self {
            context,
            detail: detail.into(),
            source: None,
        }
    }

    pub(super) fn with_source(
        context: &'static str,
        detail: impl Into<String>,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            context,
            detail: detail.into(),
            source: Some(Box::new(source)),
        }
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.detail.is_empty() {
            formatter.write_str(self.context)
        } else {
            write!(formatter, "{}: {}", self.context, self.detail)
        }
    }
}

impl Error for RunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}
