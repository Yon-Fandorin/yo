//! Bounded local image preparation; callers run this outside the UI thread.

#[cfg(unix)]
mod clipboard;
mod host;
use std::{
    fmt::Write as _,
    fs::OpenOptions,
    io::{self, Cursor, Read, Write},
    path::Path,
};

pub(crate) use host::bind;
use image::{
    DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, Limits, RgbaImage,
    codecs::{
        jpeg::JpegDecoder,
        png::{PngDecoder, PngEncoder},
    },
    imageops::{self, FilterType},
    metadata::Orientation,
};
use sha2::{Digest as _, Sha256};
use yo_core::InputImageSnapshot;

use crate::interaction::diagnostic::AppError;

const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SOURCE_SIDE: u32 = 4096;
const MAX_SOURCE_PIXELS: u64 = 4_194_304;
const MAX_THUMBNAIL_BYTES: usize = 1024 * 1024;

/// Host-observed preparation evidence, before whole-input admission.
#[derive(Debug)]
pub(crate) struct PreparedInputImage {
    snapshot: InputImageSnapshot,
    source_byte_length: u64,
    source_width: u32,
    source_height: u32,
    source_mime_type: &'static str,
    source_sha256: String,
    thumbnail_png: Vec<u8>,
}

impl PreparedInputImage {
    pub(crate) fn snapshot(&self) -> &InputImageSnapshot {
        &self.snapshot
    }
    pub(crate) fn source_byte_length(&self) -> u64 {
        self.source_byte_length
    }
    pub(crate) fn source_width(&self) -> u32 {
        self.source_width
    }
    pub(crate) fn source_height(&self) -> u32 {
        self.source_height
    }
    pub(crate) fn source_mime_type(&self) -> &str {
        self.source_mime_type
    }
    pub(crate) fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
    /// Derivative of this result's normalized snapshot, never its source file.
    pub(crate) fn thumbnail_png(&self) -> &[u8] {
        &self.thumbnail_png
    }
}

/// Reads one regular file once and prepares immutable pixels without granting admission.
pub(crate) fn prepare(
    path: &Path,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<PreparedInputImage, AppError> {
    check_cancelled(cancelled)?;
    let mut options = OpenOptions::new();
    options.read(true);
    // A FIFO selected or substituted at open must not block waiting for a writer.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = options
        .open(path)
        .map_err(|_| AppError::message("Image source could not be opened."))?;
    let metadata = file
        .metadata()
        .map_err(|_| AppError::message("Image source could not be inspected."))?;
    if !metadata.is_file() {
        return Err(AppError::message("Image source must be a regular file."));
    }
    if metadata.len() > MAX_SOURCE_BYTES as u64 {
        return Err(AppError::message("Image source exceeds the 4 MiB limit."));
    }
    let source = read_source(&mut file, cancelled)?;
    prepare_source(&source, cancelled)
}

fn read_source(
    reader: &mut impl Read,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<Vec<u8>, AppError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        check_cancelled(cancelled)?;
        // Read at most the first excess byte, including when the file grows.
        let remaining = (MAX_SOURCE_BYTES - bytes.len() + 1).min(chunk.len());
        let read = match reader.read(&mut chunk[..remaining]) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(AppError::message("Image source could not be read.")),
        };
        check_cancelled(cancelled)?;
        if read == 0 {
            return Ok(bytes);
        }
        if read > MAX_SOURCE_BYTES - bytes.len() {
            return Err(AppError::message("Image source exceeds the 4 MiB limit."));
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
}

fn prepare_source(
    source: &[u8],
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<PreparedInputImage, AppError> {
    check_cancelled(cancelled)?;
    let format = image::guess_format(source)
        .map_err(|_| AppError::message("Image source must contain a static PNG or JPEG."))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    let (decoded, source_width, source_height, source_mime_type) = match format {
        ImageFormat::Png => {
            let orientation = png_orientation(source)?;
            let decoder = PngDecoder::with_limits(Cursor::new(source), limits.clone())
                .map_err(|_| decode_error())?;
            if decoder.is_apng().map_err(|_| decode_error())? {
                return Err(AppError::message("Animated PNG images are not supported."));
            }
            let (decoded, width, height) = decode(decoder, limits, Some(orientation), cancelled)?;
            (decoded, width, height, "image/png")
        },
        ImageFormat::Jpeg => {
            let decoder = JpegDecoder::new(Cursor::new(source)).map_err(|_| decode_error())?;
            let (decoded, width, height) = decode(decoder, limits, None, cancelled)?;
            (decoded, width, height, "image/jpeg")
        },
        _ => {
            return Err(AppError::message(
                "Image source must contain a static PNG or JPEG.",
            ));
        },
    };
    check_cancelled(cancelled)?;
    // Conversion is part of the frozen profile; do not add color management.
    let pixels = decoded.to_rgba8();
    // Release any higher-bit-depth source buffer before resizing and encoding.
    drop(decoded);
    check_cancelled(cancelled)?;
    let normalized = resize(
        pixels,
        InputImageSnapshot::MAX_SIDE,
        InputImageSnapshot::MAX_PIXELS,
    )?;
    check_cancelled(cancelled)?;
    let png = encode(&normalized, InputImageSnapshot::MAX_BYTES)?;
    check_cancelled(cancelled)?;
    let snapshot = InputImageSnapshot::new(normalized.width(), normalized.height(), png)
        .map_err(|_| AppError::message("Prepared image does not satisfy snapshot limits."))?;
    let thumbnail = resize(normalized, 256, 65_536)?;
    check_cancelled(cancelled)?;
    let thumbnail_png = encode(&thumbnail, MAX_THUMBNAIL_BYTES)?;
    let mut source_sha256 = String::from("sha256:");
    for byte in Sha256::digest(source) {
        write!(source_sha256, "{byte:02x}").expect("writing into a String cannot fail");
    }
    check_cancelled(cancelled)?;
    Ok(PreparedInputImage {
        snapshot,
        source_byte_length: source.len() as u64,
        source_width,
        source_height,
        source_mime_type,
        source_sha256,
        thumbnail_png,
    })
}

fn decode(
    mut decoder: impl ImageDecoder,
    mut limits: Limits,
    orientation: Option<Orientation>,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<(DynamicImage, u32, u32), AppError> {
    let (width, height) = decoder.dimensions();
    check_dimensions(width, height, MAX_SOURCE_SIDE, MAX_SOURCE_PIXELS)?;
    limits
        .reserve(decoder.total_bytes())
        .and_then(|()| decoder.set_limits(limits))
        .map_err(|_| decode_error())?;
    check_cancelled(cancelled)?;
    let orientation = match orientation {
        Some(orientation) => orientation,
        None => decoder.orientation().map_err(|_| decode_error())?,
    };
    check_cancelled(cancelled)?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(|_| decode_error())?;
    check_cancelled(cancelled)?;
    image.apply_orientation(orientation);
    Ok((image, width, height))
}

fn png_orientation(source: &[u8]) -> Result<Orientation, AppError> {
    // image's PNG decoder exposes only metadata preceding IDAT. PNG also permits
    // eXIf after IDAT, so inspect the same bounded bytes before pixel decoding.
    let mut cursor = 8_usize;
    let mut exif_seen = false;
    let mut orientation = Orientation::NoTransforms;
    while cursor < source.len() {
        let length = source.get(cursor..cursor + 4).ok_or_else(decode_error)?;
        let length = u32::from_be_bytes(length.try_into().unwrap()) as usize;
        let end = cursor
            .checked_add(12)
            .and_then(|start| start.checked_add(length))
            .ok_or_else(decode_error)?;
        let chunk = source.get(cursor..end).ok_or_else(decode_error)?;
        match &chunk[4..8] {
            b"acTL" | b"fcTL" | b"fdAT" => {
                return Err(AppError::message("Animated PNG images are not supported."));
            },
            b"eXIf" => {
                if exif_seen
                    || png_crc32(&chunk[4..8 + length])
                        != u32::from_be_bytes(chunk[8 + length..].try_into().unwrap())
                {
                    return Err(decode_error());
                }
                exif_seen = true;
                orientation = Orientation::from_exif_chunk(&chunk[8..8 + length])
                    .unwrap_or(Orientation::NoTransforms);
            },
            b"IEND" => {
                return if length == 0 && end == source.len() {
                    Ok(orientation)
                } else {
                    Err(decode_error())
                };
            },
            _ => {},
        }
        cursor = end;
    }
    Err(decode_error())
}

fn png_crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn resize(pixels: RgbaImage, max_side: u32, max_pixels: u64) -> Result<RgbaImage, AppError> {
    let (width, height) = pixels.dimensions();
    let count = u64::from(width)
        .checked_mul(u64::from(height))
        .filter(|count| *count > 0)
        .ok_or_else(decode_error)?;
    let scale = 1_f64
        .min(f64::from(max_side) / f64::from(width.max(height)))
        .min((max_pixels as f64 / count as f64).sqrt());
    let target_width = ((f64::from(width) * scale).floor() as u32).max(1);
    let target_height = ((f64::from(height) * scale).floor() as u32).max(1);
    check_dimensions(target_width, target_height, max_side, max_pixels)?;
    if (width, height) == (target_width, target_height) {
        return Ok(pixels);
    }
    Ok(imageops::resize(
        &pixels,
        target_width,
        target_height,
        FilterType::Triangle,
    ))
}

fn check_dimensions(
    width: u32,
    height: u32,
    max_side: u32,
    max_pixels: u64,
) -> Result<(), AppError> {
    let count = u64::from(width).checked_mul(u64::from(height));
    if width == 0
        || height == 0
        || width > max_side
        || height > max_side
        || count.is_none_or(|count| count > max_pixels)
    {
        return Err(AppError::message(
            "Image dimensions exceed the preparation limits.",
        ));
    }
    Ok(())
}

fn encode(pixels: &RgbaImage, limit: usize) -> Result<Vec<u8>, AppError> {
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        limit,
        exceeded: false,
    };
    PngEncoder::new(&mut writer)
        .write_image(
            pixels.as_raw(),
            pixels.width(),
            pixels.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| AppError::message("Image PNG encoding failed or exceeded its byte limit."))?;
    // image 0.25.10 lets png::Writer write IEND during Drop, which ignores I/O
    // errors. Retain any rejected write so that a partial final chunk cannot pass.
    if writer.exceeded {
        return Err(AppError::message(
            "Image PNG encoding exceeded its byte limit.",
        ));
    }
    Ok(writer.bytes)
}

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.exceeded || bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("image PNG exceeds its byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn check_cancelled(cancelled: &mut dyn FnMut() -> bool) -> Result<(), AppError> {
    if cancelled() {
        Err(AppError::message("Image preparation was cancelled."))
    } else {
        Ok(())
    }
}

fn decode_error() -> AppError {
    AppError::message("Image could not be decoded within preparation limits.")
}

#[cfg(test)]
mod tests;
