//! Immutable image bytes shared by input and replay; no decoder or host authority.

use std::{error::Error, fmt, fmt::Write as _, sync::Arc};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser::SerializeStruct};
use sha2::{Digest as _, Sha256};

mod png;

use self::png::validate_png;

/// One normalized transmission image. Clones share the same immutable bytes.
/// Construction checks encoded identity, not source admission or normalization.
/// The execution host must still validate and prepare every live attachment.
#[derive(Clone, Eq, PartialEq)]
pub struct InputImageSnapshot {
    width: u32,
    height: u32,
    sha256: String,
    png: Arc<[u8]>,
}

/// An ordered text span or image in a model-visible user message.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelInputPart {
    Text { text: String },
    Image { snapshot: InputImageSnapshot },
}

impl ModelInputPart {
    /// Checks one image-bearing user's part and image budgets. The containing
    /// request/replay still must enforce its complete encoded JSON byte limit.
    pub fn validate_user_parts(parts: &[Self]) -> Result<(), &'static str> {
        if parts.is_empty() || parts.len() > 33 {
            return Err("image-bearing user input requires one to 33 ordered parts");
        }
        let mut images = 0_usize;
        let mut bytes = 0_usize;
        let mut previous_text = false;
        for part in parts {
            match part {
                Self::Text { text } if text.is_empty() || text.len() > 16 * 1024 * 1024 => {
                    return Err("image-bearing input text is empty or exceeds its byte limit");
                },
                Self::Text { .. } if previous_text => {
                    return Err("adjacent model input text spans must be merged");
                },
                Self::Text { .. } => previous_text = true,
                Self::Image { snapshot } => {
                    previous_text = false;
                    images += 1;
                    bytes = bytes
                        .checked_add(snapshot.png.len())
                        .ok_or("input image byte overflow")?;
                    if images > 16 || bytes > InputImageSnapshot::MAX_BYTES {
                        return Err("input image occurrence or aggregate byte limit exceeded");
                    }
                },
            }
        }
        if images == 0 {
            return Err("multimodal user input requires at least one image");
        }
        Ok(())
    }
}

/// Why a canonical image envelope cannot preserve its declared identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputImageSnapshotError {
    Profile,
    MimeType,
    Dimensions,
    ByteLength,
    Digest,
    Encoding,
}

impl InputImageSnapshot {
    /// Frozen normalization algorithm owned by the preparing execution host.
    pub const PROFILE: &str = "yo.input-image-rgba8-triangle/v1";
    /// PNG encoding of normalized RGBA8 pixels.
    pub const MIME_TYPE: &str = "image/png";
    /// Canonical encoded PNG bound, separately charged for each occurrence.
    pub const MAX_BYTES: usize = 9 * 1024 * 1024;
    /// Maximum transmitted dimension.
    pub const MAX_SIDE: u32 = 2048;
    /// Maximum transmitted pixel count.
    pub const MAX_PIXELS: u64 = 2_097_152;

    /// Captures already normalized bytes without decoding or acquiring source authority.
    /// Checks canonical RGBA8 framing, pixel-stream integrity and dimensions.
    /// Source decoding, budgets, orientation and resampling remain host duties.
    pub fn new(width: u32, height: u32, png: Vec<u8>) -> Result<Self, InputImageSnapshotError> {
        if width == 0
            || height == 0
            || width > Self::MAX_SIDE
            || height > Self::MAX_SIDE
            || u64::from(width) * u64::from(height) > Self::MAX_PIXELS
        {
            return Err(InputImageSnapshotError::Dimensions);
        }
        if png.is_empty() || png.len() > Self::MAX_BYTES {
            return Err(InputImageSnapshotError::ByteLength);
        }
        validate_png(&png, width, height)?;
        let mut sha256 = String::with_capacity(71);
        sha256.push_str("sha256:");
        for byte in Sha256::digest(&png) {
            write!(sha256, "{byte:02x}").expect("writing a digest into String cannot fail");
        }
        Ok(Self {
            width,
            height,
            sha256,
            png: png.into(),
        })
    }

    /// Transmitted width after normalization.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Transmitted height after normalization.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Exact bytes used by every provider projection and durable replay.
    pub fn png(&self) -> &[u8] {
        &self.png
    }

    /// Canonical SHA-256 spelling for the exact transmitted bytes.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

impl fmt::Debug for InputImageSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputImageSnapshot")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("byte_length", &self.png.len())
            .field("sha256", &self.sha256)
            .finish()
    }
}

impl fmt::Display for InputImageSnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Profile => "unsupported input image normalization profile",
            Self::MimeType => "input image snapshot must contain PNG",
            Self::Dimensions => "input image dimensions are inconsistent or exceed the limit",
            Self::ByteLength => "input image byte length is inconsistent or exceeds the limit",
            Self::Digest => "input image digest does not match its bytes",
            Self::Encoding => "invalid canonical input image encoding",
        })
    }
}

impl Error for InputImageSnapshotError {}

impl Serialize for InputImageSnapshot {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut wire = serializer.serialize_struct("InputImageSnapshot", 7)?;
        wire.serialize_field("profile", Self::PROFILE)?;
        wire.serialize_field("mime_type", Self::MIME_TYPE)?;
        wire.serialize_field("width", &self.width)?;
        wire.serialize_field("height", &self.height)?;
        wire.serialize_field("byte_length", &self.png.len())?;
        wire.serialize_field("sha256", &self.sha256)?;
        wire.serialize_field("data_base64", &STANDARD.encode(&self.png))?;
        wire.end()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSnapshot {
    profile: String,
    mime_type: String,
    width: u32,
    height: u32,
    byte_length: u64,
    sha256: String,
    data_base64: EncodedPng,
}

struct EncodedPng(Vec<u8>);

impl<'de> Deserialize<'de> for EncodedPng {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl de::Visitor<'_> for Visitor {
            type Value = EncodedPng;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("bounded canonical padded base64 for an input PNG")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value.len() > InputImageSnapshot::MAX_BYTES.div_ceil(3) * 4 {
                    return Err(E::custom(InputImageSnapshotError::ByteLength));
                }
                let bytes = STANDARD
                    .decode(value)
                    .map_err(|_| E::custom(InputImageSnapshotError::Encoding))?;
                if bytes.len() > InputImageSnapshot::MAX_BYTES {
                    return Err(E::custom(InputImageSnapshotError::ByteLength));
                }
                Ok(EncodedPng(bytes))
            }
        }
        decoder.deserialize_str(Visitor)
    }
}

impl<'de> Deserialize<'de> for InputImageSnapshot {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let wire = WireSnapshot::deserialize(decoder)?;
        let result = (|| {
            if wire.profile != Self::PROFILE {
                return Err(InputImageSnapshotError::Profile);
            }
            if wire.mime_type != Self::MIME_TYPE {
                return Err(InputImageSnapshotError::MimeType);
            }
            if wire.byte_length != wire.data_base64.0.len() as u64 {
                return Err(InputImageSnapshotError::ByteLength);
            }
            let snapshot = Self::new(wire.width, wire.height, wire.data_base64.0)?;
            if snapshot.sha256 != wire.sha256 {
                return Err(InputImageSnapshotError::Digest);
            }
            Ok(snapshot)
        })();
        result.map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests;
