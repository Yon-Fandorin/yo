//! Limits apply to the complete request, including retained and repeated media.

use std::io::{self, Write};

use serde_json::Value;
use yo_core::{ConnectorError, ModelConnectorRequest};

use super::configuration_failure;

const MAX_IMAGES: u64 = 250;
const MAX_PNG_BYTES: u64 = 16_777_216;
const MAX_JSON_BYTES: u64 = 33_554_432;

pub(super) fn validate_media(request: &ModelConnectorRequest) -> Result<(), ConnectorError> {
    let mut images = 0_u64;
    let mut bytes = 0_u64;
    for image in request.input_images() {
        images = images.checked_add(1).ok_or_else(media_failure)?;
        bytes = bytes
            .checked_add(u64::try_from(image.png().len()).map_err(|_| media_failure())?)
            .ok_or_else(media_failure)?;
        if images > MAX_IMAGES || bytes > MAX_PNG_BYTES {
            return Err(media_failure());
        }
    }
    Ok(())
}

fn media_failure() -> ConnectorError {
    configuration_failure(
        "complete Qwen image request exceeds 250 PNG occurrences or 16 MiB PNG bytes",
    )
}

pub(super) fn validate_encoded_body(body: &Value) -> Result<(), ConnectorError> {
    serde_json::to_writer(&mut EncodedSize(0), body).map_err(|_| {
        configuration_failure("complete Qwen request exceeds 32 MiB encoded JSON bytes")
    })
}

/// Measure exact JSON encoding without allocating another complete request buffer.
struct EncodedSize(u64);

impl Write for EncodedSize {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let length = u64::try_from(bytes.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
        self.0 = self
            .0
            .checked_add(length)
            .filter(|size| *size <= MAX_JSON_BYTES)
            .ok_or(io::ErrorKind::InvalidInput)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
