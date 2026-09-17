use super::{
    error::UserInputError,
    model::{InputReference, UserInput},
};
use crate::{InputImage, InputImageSnapshot};

impl InputReference {
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
            } => visible == &super::super::projection::workspace_reference_projection(reference),
            Self::Skill {
                projection: visible,
                reference,
                ..
            } => visible == &super::super::projection::skill_reference_projection(reference),
        }
    }
}

impl UserInput {
    pub(super) fn validate(&self) -> Result<(), UserInputError> {
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

pub(super) fn validate_image_occurrences(
    text: &str,
    references: &[InputReference],
    images: &[InputImage],
) -> Result<(), UserInputError> {
    if images.len() > InputImage::MAX_OCCURRENCES {
        return Err(UserInputError::ImageBudgetExceeded);
    }
    let mut previous_end = 0;
    let mut source_bytes = 0_u64;
    let mut png_bytes = 0_usize;
    for (index, image) in images.iter().enumerate() {
        let span = image.span();
        if span.start >= span.end
            || span.start < previous_end
            || text.get(span.clone()) != Some(InputImage::PROJECTION)
            || references.iter().any(|reference| {
                reference.span().start < span.end && span.start < reference.span().end
            })
        {
            return Err(UserInputError::InvalidImage { index });
        }
        source_bytes = source_bytes
            .checked_add(image.source_byte_length())
            .ok_or(UserInputError::ImageBudgetExceeded)?;
        png_bytes = png_bytes
            .checked_add(image.snapshot().png().len())
            .ok_or(UserInputError::ImageBudgetExceeded)?;
        if source_bytes > InputImage::MAX_INPUT_SOURCE_BYTES
            || png_bytes > InputImageSnapshot::MAX_BYTES
        {
            return Err(UserInputError::ImageBudgetExceeded);
        }
        previous_end = span.end;
    }
    Ok(())
}
