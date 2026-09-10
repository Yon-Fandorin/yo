use std::{fmt, ops::Range};

use super::{InputImage, projection};
use crate::{ModelInputPart, SkillReference, WorkspaceReference};

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
    images: Vec<InputImage>,
    resolved_skill: Option<Box<ResolvedSkill>>,
    model_text: Option<String>,
}

/// Immutable instructions assembled by the execution host for one selected skill.
/// Includes any required supporting instructions; optional assets remain host-owned and lazy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSkill {
    reference: SkillReference,
    instructions: String,
}

impl ResolvedSkill {
    /// Maximum UTF-8 instruction bytes, before request framing and model context accounting.
    pub const MAX_INSTRUCTION_BYTES: usize = 256 * 1024;

    /// Captures instructions from the same snapshot whose identity and eligibility were validated.
    /// This constructor performs no filesystem access or authorization.
    pub fn new(
        reference: SkillReference,
        instructions: impl Into<String>,
    ) -> Result<Self, UserInputError> {
        let instructions = instructions.into();
        // This set is frozen by the persisted v2 profile, independently of Unicode updates.
        let blank = instructions.chars().all(|character| {
            matches!(character,
                '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{0085}' | '\u{00a0}'
                | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}'
                | '\u{202f}' | '\u{205f}' | '\u{3000}'
            )
        });
        if blank || instructions.len() > Self::MAX_INSTRUCTION_BYTES {
            return Err(UserInputError::InvalidSkillInstructions);
        }
        Ok(Self {
            reference,
            instructions,
        })
    }

    #[must_use]
    pub const fn reference(&self) -> &SkillReference {
        &self.reference
    }

    #[must_use]
    pub fn instructions(&self) -> &str {
        &self.instructions
    }
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
    InvalidSkillInstructions,
    SkillSnapshotMismatch,
    InvalidImage { index: usize },
    InvalidImageSourceLength,
    InvalidImageDisplay,
    ImageBudgetExceeded,
    EncodedImageInputTooLarge,
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

    pub(crate) fn persisted_workspace(
        span: Range<usize>,
        projection: String,
        reference: WorkspaceReference,
    ) -> Self {
        Self::Workspace {
            span,
            projection,
            reference,
        }
    }

    pub(crate) fn persisted_skill(
        span: Range<usize>,
        projection: String,
        reference: SkillReference,
    ) -> Self {
        Self::Skill {
            span,
            projection,
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

    pub(crate) fn projection(&self) -> &str {
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
            images: Vec::new(),
            resolved_skill: None,
            model_text: None,
        }
    }

    pub fn with_references(
        text: impl Into<String>,
        references: Vec<InputReference>,
    ) -> Result<Self, UserInputError> {
        let input = Self {
            text: text.into(),
            references,
            images: Vec::new(),
            resolved_skill: None,
            model_text: None,
        };
        input.validate()?;
        Ok(input)
    }

    pub(crate) fn from_validated_persisted_v1(
        text: String,
        references: Vec<InputReference>,
    ) -> Self {
        Self {
            text,
            references,
            images: Vec::new(),
            resolved_skill: None,
            model_text: None,
        }
    }

    /// Visible draft text without execution-host instruction context.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn references(&self) -> &[InputReference] {
        &self.references
    }

    /// Consumes the visible draft. Provider adapters use `into_model_input` instead.
    #[must_use]
    pub fn into_string(self) -> String {
        self.text
    }

    /// Adds a host-resolved snapshot while preserving the visible input and reference spans.
    /// Live runtime admission never trusts a caller-supplied snapshot as authorization.
    pub fn with_resolved_skill(mut self, skill: ResolvedSkill) -> Result<Self, UserInputError> {
        let mut selected = self
            .references
            .iter()
            .filter_map(InputReference::skill_reference);
        if self.resolved_skill.is_some()
            || selected.next() != Some(&skill.reference)
            || selected.next().is_some()
        {
            return Err(UserInputError::SkillSnapshotMismatch);
        }
        // Frozen v2 input framing: recovery must reproduce the exact model-visible request.
        let snapshot = serde_json::json!({
            "name": skill.reference.name(),
            "source": skill.reference.locator(),
            "instructions": skill.instructions,
        });
        self.model_text = Some(format!(
            "{}\n\nExplicit skill instructions (yo.skill-instructions/v1):\n{}",
            self.text, snapshot
        ));
        self.resolved_skill = Some(Box::new(skill));
        self.validate_image_encoding()?;
        Ok(self)
    }

    #[must_use]
    pub fn resolved_skill(&self) -> Option<&ResolvedSkill> {
        self.resolved_skill.as_deref()
    }

    /// Exact user-role content for provider requests, context accounting, and replay.
    /// Unresolved and historical v1 inputs retain their original text byte-for-byte.
    #[must_use]
    pub fn model_input(&self) -> &str {
        assert!(
            self.images.is_empty(),
            "image input requires ordered model_parts"
        );
        self.model_text.as_deref().unwrap_or(&self.text)
    }

    #[must_use]
    pub fn into_model_input(self) -> String {
        assert!(
            self.images.is_empty(),
            "image input requires ordered model_parts"
        );
        self.model_text.unwrap_or(self.text)
    }

    /// Attaches exact image occurrences, preserving separately validated reference projections.
    /// Source lengths are retained metadata; execution-host admission must validate live evidence.
    pub fn with_images(mut self, images: Vec<InputImage>) -> Result<Self, UserInputError> {
        InputImage::validate_occurrences(&self.text, &self.references, &images)?;
        self.images = images;
        self.validate_image_encoding()?;
        Ok(self)
    }

    /// Ordered image occurrences, including original compressed source-byte charges.
    #[must_use]
    pub fn images(&self) -> &[InputImage] {
        &self.images
    }

    /// Exact ordered model content; visible attachment markers have no text projection.
    #[must_use]
    pub fn model_parts(&self) -> Vec<ModelInputPart> {
        if self.images.is_empty() {
            return vec![ModelInputPart::Text {
                text: self.model_input().to_owned(),
            }];
        }
        let mut parts = Vec::with_capacity(self.images.len() * 2 + 1);
        let mut cursor = 0;
        for image in &self.images {
            if image.span().start > cursor {
                parts.push(ModelInputPart::Text {
                    text: self.text[cursor..image.span().start].to_owned(),
                });
            }
            parts.push(ModelInputPart::Image {
                snapshot: image.snapshot().clone(),
            });
            cursor = image.span().end;
        }
        if cursor < self.text.len() {
            parts.push(ModelInputPart::Text {
                text: self.text[cursor..].to_owned(),
            });
        }
        if let Some(model_text) = &self.model_text {
            let trailer = &model_text[self.text.len()..];
            if let Some(ModelInputPart::Text { text }) = parts.last_mut() {
                text.push_str(trailer);
            } else {
                parts.push(ModelInputPart::Text {
                    text: trailer.to_owned(),
                });
            }
        }
        parts
    }

    /// Exact user-role replay item, preserving legacy string messages for text-only input.
    #[must_use]
    pub fn model_replay_item(&self) -> crate::ModelReplayItem {
        if self.images.is_empty() {
            crate::ModelReplayItem::Message {
                role: crate::ModelReplayRole::User,
                content: self.model_input().to_owned(),
                refusal: None,
            }
        } else {
            crate::ModelReplayItem::MultimodalUser {
                parts: self.model_parts(),
            }
        }
    }

    fn validate_image_encoding(&self) -> Result<(), UserInputError> {
        if !self.images.is_empty() {
            crate::journal::codec::validate_image_input_encoding(self)
                .map_err(|_| UserInputError::EncodedImageInputTooLarge)?;
        }
        Ok(())
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
            Self::InvalidImage { index } => write!(
                formatter,
                "input image {index} has an invalid or overlapping visible span"
            ),
            Self::InvalidImageSourceLength => {
                formatter.write_str("image source byte length must be positive and at most 4 MiB")
            },
            Self::InvalidImageDisplay => {
                formatter.write_str("image display metadata is invalid or exceeds its bounds")
            },
            Self::ImageBudgetExceeded => formatter
                .write_str("input exceeds its image occurrence, source-byte or PNG-byte budget"),
            Self::EncodedImageInputTooLarge => {
                formatter.write_str("complete encoded image input exceeds 16 MiB")
            },
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
            Self::InvalidSkillInstructions => {
                formatter.write_str("skill instructions must be nonempty and at most 256 KiB")
            },
            Self::SkillSnapshotMismatch => {
                formatter.write_str("resolved skill does not match the unique selected reference")
            },
            Self::TooManySkills => {
                formatter.write_str("version 1 accepts at most one explicit skill")
            },
        }
    }
}

impl std::error::Error for UserInputError {}
