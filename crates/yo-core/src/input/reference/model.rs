use std::ops::Range;

use super::super::InputImage;
use crate::{SkillReference, WorkspaceReference};

/// 표시 입력의 정확한 UTF-8 byte span에 결합된 typed reference입니다.
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

/// 제출된 의미 입력이며 reference 순서는 표시 초안 순서와 같습니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserInput {
    pub(super) text: String,
    pub(super) references: Vec<InputReference>,
    pub(super) images: Vec<InputImage>,
    pub(super) resolved_skill: Option<Box<ResolvedSkill>>,
    pub(super) model_text: Option<String>,
}

/// 선택된 skill을 위해 실행 host가 조립한 불변 instruction snapshot입니다.
/// 필요한 지원 instruction을 포함하며 선택적 asset은 host가 지연 소유합니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSkill {
    pub(super) reference: SkillReference,
    pub(super) instructions: String,
}

impl InputReference {
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
}

impl ResolvedSkill {
    /// request framing과 model context accounting 전의 최대 UTF-8 instruction byte입니다.
    pub const MAX_INSTRUCTION_BYTES: usize = 256 * 1024;

    #[must_use]
    pub const fn reference(&self) -> &SkillReference {
        &self.reference
    }

    #[must_use]
    pub fn instructions(&self) -> &str {
        &self.instructions
    }
}

impl UserInput {
    /// execution host instruction context를 포함하지 않은 표시 초안입니다.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn references(&self) -> &[InputReference] {
        &self.references
    }

    /// provider adapter가 model input 대신 사용하는 표시 초안을 소비합니다.
    #[must_use]
    pub fn into_string(self) -> String {
        self.text
    }

    #[must_use]
    pub fn resolved_skill(&self) -> Option<&ResolvedSkill> {
        self.resolved_skill.as_deref()
    }

    /// 원본 압축 source byte charge를 포함한 순서 있는 image occurrence입니다.
    #[must_use]
    pub fn images(&self) -> &[InputImage] {
        &self.images
    }
}
