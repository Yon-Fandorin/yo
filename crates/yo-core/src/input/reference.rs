use std::{fmt, ops::Range};

use super::projection;
use crate::{SkillReference, WorkspaceReference};

/// One typed reference attached to an exact byte span in the visible input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputReference {
    Workspace {
        span: Range<usize>,
        projection: String,
        reference: WorkspaceReference,
    },
    Skill {
        span: Range<usize>,
        projection: String,
        reference: SkillReference,
    },
}

/// A submitted semantic input. Reference order is the visible draft order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserInput {
    text: String,
    references: Vec<InputReference>,
}

/// Why text and typed reference occurrences cannot form one honest input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserInputError {
    EmptyReferenceSpan { index: usize },
    InvalidReferenceBoundary { index: usize },
    ReferenceOrder { index: usize },
    ProjectionMismatch { index: usize },
    InvalidReferenceMetadata { index: usize },
    TooManySkills,
}

impl InputReference {
    #[must_use]
    pub fn workspace(span: Range<usize>, reference: WorkspaceReference) -> Self {
        let visible_projection = projection::workspace_reference_projection(&reference);
        Self::Workspace {
            span,
            projection: visible_projection,
            reference,
        }
    }

    #[must_use]
    pub fn skill(span: Range<usize>, reference: SkillReference) -> Self {
        let visible_projection = projection::skill_reference_projection(&reference);
        Self::Skill {
            span,
            projection: visible_projection,
            reference,
        }
    }

    #[must_use]
    pub fn span(&self) -> &Range<usize> {
        match self {
            Self::Workspace { span, .. } | Self::Skill { span, .. } => span,
        }
    }

    #[must_use]
    pub fn workspace_reference(&self) -> Option<&WorkspaceReference> {
        match self {
            Self::Workspace { reference, .. } => Some(reference),
            Self::Skill { .. } => None,
        }
    }

    #[must_use]
    pub fn skill_reference(&self) -> Option<&SkillReference> {
        match self {
            Self::Skill { reference, .. } => Some(reference),
            Self::Workspace { .. } => None,
        }
    }

    fn projection(&self) -> &str {
        match self {
            Self::Workspace { projection, .. } | Self::Skill { projection, .. } => projection,
        }
    }

    fn has_valid_metadata(&self) -> bool {
        match self {
            Self::Workspace { reference, .. } => {
                !reference.identity().is_empty()
                    && !reference.execution_environment_identity().is_empty()
                    && !reference.workspace_identity().is_empty()
                    && !reference.root_identity().is_empty()
            },
            Self::Skill { reference, .. } => {
                !reference.identity().is_empty()
                    && !reference.execution_environment_identity().is_empty()
                    && !reference.locator().is_empty()
                    && !reference.name().is_empty()
                    && reference.catalog_generation() > 0
                    && !reference.entry_revision().is_empty()
            },
        }
    }

    fn has_canonical_projection(&self) -> bool {
        match self {
            Self::Workspace {
                projection: visible,
                reference,
                ..
            } => visible == &projection::workspace_reference_projection(reference),
            Self::Skill {
                projection: visible,
                reference,
                ..
            } => visible == &projection::skill_reference_projection(reference),
        }
    }
}

impl UserInput {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            references: Vec::new(),
        }
    }

    pub fn with_references(
        text: impl Into<String>,
        references: Vec<InputReference>,
    ) -> Result<Self, UserInputError> {
        let input = Self {
            text: text.into(),
            references,
        };
        input.validate()?;
        Ok(input)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn references(&self) -> &[InputReference] {
        &self.references
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.text
    }

    fn validate(&self) -> Result<(), UserInputError> {
        let mut previous_end = 0;
        let mut skill_count = 0;
        for (index, occurrence) in self.references.iter().enumerate() {
            let span = occurrence.span();
            if span.start >= span.end {
                return Err(UserInputError::EmptyReferenceSpan { index });
            }
            if span.end > self.text.len()
                || !self.text.is_char_boundary(span.start)
                || !self.text.is_char_boundary(span.end)
            {
                return Err(UserInputError::InvalidReferenceBoundary { index });
            }
            if index > 0 && span.start < previous_end {
                return Err(UserInputError::ReferenceOrder { index });
            }
            if !occurrence.has_canonical_projection()
                || self.text.get(span.clone()) != Some(occurrence.projection())
            {
                return Err(UserInputError::ProjectionMismatch { index });
            }
            if !occurrence.has_valid_metadata() {
                return Err(UserInputError::InvalidReferenceMetadata { index });
            }
            if matches!(occurrence, InputReference::Skill { .. }) {
                skill_count += 1;
            }
            previous_end = span.end;
        }
        if skill_count > 1 {
            return Err(UserInputError::TooManySkills);
        }
        Ok(())
    }
}

impl From<String> for UserInput {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl From<&str> for UserInput {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl fmt::Display for UserInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyReferenceSpan { index } => {
                write!(formatter, "input reference {index} has an empty span")
            },
            Self::InvalidReferenceBoundary { index } => write!(
                formatter,
                "input reference {index} is outside text or splits a UTF-8 character"
            ),
            Self::ReferenceOrder { index } => write!(
                formatter,
                "input reference {index} overlaps or precedes an earlier reference"
            ),
            Self::ProjectionMismatch { index } => write!(
                formatter,
                "input reference {index} does not match its visible projection"
            ),
            Self::InvalidReferenceMetadata { index } => write!(
                formatter,
                "input reference {index} is missing required identity or revision metadata"
            ),
            Self::TooManySkills => {
                formatter.write_str("version 1 accepts at most one explicit skill")
            },
        }
    }
}

impl std::error::Error for UserInputError {}
