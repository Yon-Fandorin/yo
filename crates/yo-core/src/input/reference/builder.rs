use std::ops::Range;

use super::{
    error::UserInputError,
    model::{InputReference, ResolvedSkill, UserInput},
    validation,
};
use crate::{InputImage, SkillReference, WorkspaceReference};

impl InputReference {
    /// workspace identity에서 표시용 reference occurrence를 만듭니다.
    #[must_use]
    pub fn workspace(span: Range<usize>, reference: WorkspaceReference) -> Self {
        let visible_projection =
            super::super::projection::workspace_reference_projection(&reference);
        Self::Workspace {
            span,
            projection: visible_projection,
            reference,
        }
    }

    /// skill identity에서 표시용 reference occurrence를 만듭니다.
    #[must_use]
    pub fn skill(span: Range<usize>, reference: SkillReference) -> Self {
        let visible_projection = super::super::projection::skill_reference_projection(&reference);
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
}

impl ResolvedSkill {
    /// 동일한 snapshot에서 검증된 skill instruction을 캡처합니다.
    /// filesystem 접근이나 authorization은 수행하지 않습니다.
    pub fn new(
        reference: SkillReference,
        instructions: impl Into<String>,
    ) -> Result<Self, UserInputError> {
        let instructions = instructions.into();
        // 이 whitespace 집합은 Unicode 분류 변경과 무관하게 v2 profile에 고정됩니다.
        let blank = instructions.chars().all(|character| {
            matches!(
                character,
                '\u{0009}'..='\u{000d}'
                    | '\u{0020}'
                    | '\u{0085}'
                    | '\u{00a0}'
                    | '\u{1680}'
                    | '\u{2000}'..='\u{200a}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202f}'
                    | '\u{205f}'
                    | '\u{3000}'
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
}

impl UserInput {
    /// 표시 text만 포함한 새 입력을 만듭니다.
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

    /// 표시 text와 typed reference occurrence를 검증한 입력을 만듭니다.
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

    /// 표시 marker occurrence에 준비된 image snapshot을 결합합니다.
    /// source length는 metadata로 남으며 live admission이 실행 증거를 검증합니다.
    pub fn with_images(mut self, images: Vec<InputImage>) -> Result<Self, UserInputError> {
        validation::validate_image_occurrences(&self.text, &self.references, &images)?;
        self.images = images;
        self.validate_image_encoding()?;
        Ok(self)
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
