//! Visible image occurrences and bounded, non-model source display metadata.

use std::ops::Range;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use super::{InputReference, UserInputError};
use crate::InputImageSnapshot;

/// One immutable attachment bound to its visible marker and original source-byte charge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputImage {
    span: Range<usize>,
    source_byte_length: u64,
    snapshot: InputImageSnapshot,
    display: Option<InputImageDisplay>,
}

/// Optional source facts for display, excluded from model and replay authority.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputImageDisplay {
    /// Original basename without directory or control characters, at most 255 UTF-8 bytes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub filename: Option<String>,
    /// Original content-verified PNG or JPEG MIME type.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub source_mime_type: Option<String>,
    /// Original positive header width, at most 4096 pixels.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub source_width: Option<u32>,
    /// Original positive header height, at most 4096 pixels.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub source_height: Option<u32>,
    /// SHA-256 of the original compressed source bytes, in canonical spelling.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub source_sha256: Option<String>,
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    decoder: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(decoder).map(Some)
}

impl InputImageDisplay {
    fn validate(&self) -> Result<(), UserInputError> {
        let valid = self.filename.as_ref().is_none_or(|name| {
            !name.is_empty()
                && name.len() <= 255
                && name != "."
                && name != ".."
                && !name
                    .chars()
                    .any(|c| c == '/' || c == '\\' || c.is_control())
        }) && self
            .source_mime_type
            .as_deref()
            .is_none_or(|mime| matches!(mime, "image/png" | "image/jpeg"))
            && self
                .source_width
                .is_none_or(|side| side > 0 && side <= 4096)
            && self
                .source_height
                .is_none_or(|side| side > 0 && side <= 4096)
            && self
                .source_width
                .zip(self.source_height)
                .is_none_or(|(w, h)| u64::from(w) * u64::from(h) <= 4_194_304)
            && self.source_sha256.as_ref().is_none_or(|hash| {
                hash.len() == 71
                    && hash.starts_with("sha256:")
                    && hash.as_bytes()[7..]
                        .iter()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
            });
        if valid {
            Ok(())
        } else {
            Err(UserInputError::InvalidImageDisplay)
        }
    }
}

impl InputImage {
    /// Exact visible marker; literal unannotated text is never an attachment.
    pub const PROJECTION: &str = "[image]";
    /// Maximum compressed source bytes per occurrence.
    pub const MAX_SOURCE_BYTES: u64 = 4_194_304;
    /// Maximum compressed source bytes summed per occurrence in one input.
    pub const MAX_INPUT_SOURCE_BYTES: u64 = 8_388_608;
    /// Maximum distinct occurrences in one input, including repeated bytes.
    pub const MAX_OCCURRENCES: usize = 16;
    /// Maximum complete canonical v3 input encoding, including framing and escaping.
    pub const MAX_ENCODED_INPUT_BYTES: usize = 16_777_216;

    /// Binds a prepared snapshot to its original source charge. Whole-input span
    /// and aggregate checks occur in `UserInput::with_images`.
    pub fn new(
        span: Range<usize>,
        source_byte_length: u64,
        snapshot: InputImageSnapshot,
    ) -> Result<Self, UserInputError> {
        if source_byte_length == 0 || source_byte_length > Self::MAX_SOURCE_BYTES {
            return Err(UserInputError::InvalidImageSourceLength);
        }
        Ok(Self {
            span,
            source_byte_length,
            snapshot,
            display: None,
        })
    }

    /// Adds bounded display facts without changing model-visible image identity.
    pub fn with_display(mut self, display: InputImageDisplay) -> Result<Self, UserInputError> {
        display.validate()?;
        self.display = Some(display);
        Ok(self)
    }

    /// Exact UTF-8 byte span occupied by the visible marker.
    pub const fn span(&self) -> &Range<usize> {
        &self.span
    }

    /// Original bounded compressed source length, charged per occurrence.
    pub const fn source_byte_length(&self) -> u64 {
        self.source_byte_length
    }

    /// Immutable normalized PNG used by all model and persistence consumers.
    pub const fn snapshot(&self) -> &InputImageSnapshot {
        &self.snapshot
    }

    /// Optional source facts; never part of model projection.
    pub const fn display(&self) -> Option<&InputImageDisplay> {
        self.display.as_ref()
    }

    pub(super) fn validate_occurrences(
        text: &str,
        references: &[InputReference],
        images: &[Self],
    ) -> Result<(), UserInputError> {
        if images.len() > Self::MAX_OCCURRENCES {
            return Err(UserInputError::ImageBudgetExceeded);
        }
        let mut previous_end = 0;
        let mut source_bytes = 0_u64;
        let mut png_bytes = 0_usize;
        for (index, image) in images.iter().enumerate() {
            let span = image.span();
            if span.start >= span.end
                || span.start < previous_end
                || text.get(span.clone()) != Some(Self::PROJECTION)
                || references.iter().any(|reference| {
                    reference.span().start < span.end && span.start < reference.span().end
                })
            {
                return Err(UserInputError::InvalidImage { index });
            }
            source_bytes = source_bytes
                .checked_add(image.source_byte_length)
                .ok_or(UserInputError::ImageBudgetExceeded)?;
            png_bytes = png_bytes
                .checked_add(image.snapshot.png().len())
                .ok_or(UserInputError::ImageBudgetExceeded)?;
            if source_bytes > Self::MAX_INPUT_SOURCE_BYTES
                || png_bytes > InputImageSnapshot::MAX_BYTES
            {
                return Err(UserInputError::ImageBudgetExceeded);
            }
            previous_end = span.end;
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireInputImage {
    start: u64,
    end: u64,
    projection: String,
    source_byte_length: u64,
    snapshot: InputImageSnapshot,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    display: Option<InputImageDisplay>,
}

impl Serialize for InputImage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error as _;
        WireInputImage {
            start: u64::try_from(self.span.start).map_err(S::Error::custom)?,
            end: u64::try_from(self.span.end).map_err(S::Error::custom)?,
            projection: Self::PROJECTION.to_owned(),
            source_byte_length: self.source_byte_length,
            snapshot: self.snapshot.clone(),
            display: self.display.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for InputImage {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let wire = WireInputImage::deserialize(decoder)?;
        if wire.projection != Self::PROJECTION {
            return Err(de::Error::custom("invalid image projection"));
        }
        let start = usize::try_from(wire.start).map_err(de::Error::custom)?;
        let end = usize::try_from(wire.end).map_err(de::Error::custom)?;
        let image = Self::new(start..end, wire.source_byte_length, wire.snapshot)
            .map_err(de::Error::custom)?;
        match wire.display {
            Some(display) => image.with_display(display).map_err(de::Error::custom),
            None => Ok(image),
        }
    }
}

#[cfg(test)]
mod tests;
