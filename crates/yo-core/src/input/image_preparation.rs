//! Frontend-neutral local image preparation; prepared values do not grant admission.

use std::{
    path::PathBuf,
    sync::Arc,
    task::{Context, Poll},
};

use super::{InputImage, SubmissionRejection, SubmissionRejectionKind};

/// Source selected by an explicit attachment gesture; bytes are acquired by the host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImagePreparationSource {
    /// File in the execution host's filesystem.
    File(PathBuf),
    /// The execution host's explicitly configured clipboard source.
    Clipboard,
}

/// Explicit source selection correlated to one frontend draft revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImagePreparationRequest {
    pub id: u64,
    pub revision: u64,
    pub source: ImagePreparationSource,
}

/// Immutable transmission image and its host-prepared display derivative.
#[derive(Clone, Eq, PartialEq)]
pub struct PreparedImageAttachment {
    image: InputImage,
    thumbnail_png: Arc<[u8]>,
    thumbnail_width: u32,
    thumbnail_height: u32,
}

impl std::fmt::Debug for PreparedImageAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedImageAttachment")
            .field("image", &self.image)
            .field("thumbnail_bytes", &self.thumbnail_png.len())
            .finish()
    }
}

impl PreparedImageAttachment {
    /// Captures a bounded derivative bound to this immutable image's digest.
    /// Live authorization remains with the execution host that prepared the pixels.
    pub fn new(
        image: InputImage,
        png: Vec<u8>,
        width: u32,
        height: u32,
    ) -> Result<Self, SubmissionRejection> {
        if width == 0
            || height == 0
            || width > 256
            || height > 256
            || u64::from(width) * u64::from(height) > 65_536
            || width > image.snapshot().width()
            || height > image.snapshot().height()
            || png.len() > 1_048_576
            || png.get(..8) != Some(b"\x89PNG\r\n\x1a\n")
            || png.get(16..20) != Some(width.to_be_bytes().as_slice())
            || png.get(20..24) != Some(height.to_be_bytes().as_slice())
        {
            return Err(SubmissionRejection::new(
                SubmissionRejectionKind::OverBudget,
                "Prepared image thumbnail is invalid or exceeds its bounds.",
            ));
        }
        Ok(Self {
            image,
            thumbnail_png: png.into(),
            thumbnail_width: width,
            thumbnail_height: height,
        })
    }
    pub fn image(&self) -> &InputImage {
        &self.image
    }
    pub fn thumbnail_png(&self) -> &Arc<[u8]> {
        &self.thumbnail_png
    }
    pub const fn thumbnail_width(&self) -> u32 {
        self.thumbnail_width
    }
    pub const fn thumbnail_height(&self) -> u32 {
        self.thumbnail_height
    }
    pub fn snapshot_digest(&self) -> &str {
        self.image.snapshot().sha256()
    }
}

/// One terminal preparation result, always carrying the original draft identity.
#[derive(Debug)]
pub struct ImagePreparationUpdate {
    pub id: u64,
    pub revision: u64,
    pub result: Result<PreparedImageAttachment, SubmissionRejection>,
}

/// One execution host's bounded preparation lane. No method decodes on the UI thread.
pub trait ImagePreparationHost: Send {
    fn start(&mut self, request: ImagePreparationRequest) -> Result<(), SubmissionRejection>;
    fn poll(&mut self, context: &mut Context<'_>) -> Poll<ImagePreparationUpdate>;
    fn cancel(&mut self);
}
