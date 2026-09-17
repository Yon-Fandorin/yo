use std::{error::Error, fmt};

/// 표시 text와 typed reference가 하나의 유효한 입력을 이루지 못한 이유입니다.
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

impl Error for UserInputError {}
