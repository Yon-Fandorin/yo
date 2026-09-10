//! Bounded image decoding, thumbnail cache and cell fallback.
use std::{
    collections::VecDeque,
    io::Cursor,
    num::NonZeroU16,
    sync::{Arc, Mutex, OnceLock},
};

use ::image::{
    DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, RgbaImage,
    imageops::{FilterType, resize},
    metadata::Orientation,
};
use base64::{Engine, engine::general_purpose::STANDARD};

use super::{Block, BlockFormat, Decoration, Role, media_text};
use crate::surface::{Color, Point, RasterImage, Rect, Size};

pub(super) fn pixel_color(pixel: [u8; 3], capability: Color) -> Color {
    match capability {
        Color::Rgb { .. } => Color::Rgb {
            red: pixel[0],
            green: pixel[1],
            blue: pixel[2],
        },
        Color::Indexed(_) => {
            let component = |value: u8| ((u16::from(value) * 5 + 127) / 255) as u8;
            Color::Indexed(
                16 + 36 * component(pixel[0]) + 6 * component(pixel[1]) + component(pixel[2]),
            )
        },
        _ => Color::Default,
    }
}

pub(super) fn image_blocks(
    source: &str,
    alt: &str,
    context: Block,
    width: NonZeroU16,
    max_width: NonZeroU16,
) -> Vec<Block> {
    let decoded = decode_image(source);
    let title = if alt.is_empty() { "Image" } else { alt };
    let image = match decoded {
        Ok(image) => image,
        Err(reason) => {
            // Never flood the chat with an encoded payload or fetch remote/local resources
            // from layout. The original destination remains in stored Markdown.
            let destination = if source.starts_with("data:") {
                "embedded image"
            } else {
                source
            };
            return vec![media_text(
                format!("Image · {title}\n{reason}\n{destination}"),
                &context,
                Role::Quote,
            )];
        },
    };
    let original = image.original;
    let png = image.png.clone();
    let available = width
        .get()
        .saturating_sub(context.prefix.chars().count() as u16)
        .clamp(1, max_width.get().min(64));
    let image = if u32::from(available) < image.pixels.width() {
        let height = ((u64::from(image.pixels.height()) * u64::from(available))
            / u64::from(image.pixels.width()))
        .max(1) as u32;
        resize(
            &image.pixels,
            u32::from(available),
            height,
            FilterType::Triangle,
        )
    } else {
        image.pixels
    };
    let mut result = vec![media_text(
        format!("Image · {title} · {} × {}", original.0, original.1),
        &context,
        Role::Heading,
    )];
    for y in (0..image.height()).step_by(2) {
        let mut row = media_text(String::new(), &context, Role::Body);
        row.format = BlockFormat::TableRow;
        for x in 0..image.width() {
            let composite = |pixel: &[u8; 4]| {
                let alpha = u16::from(pixel[3]);
                // A neutral checkerboard makes transparency explicit and host-independent.
                let base = if (x / 4 + y / 4) % 2 == 0 { 184 } else { 216 };
                [0, 1, 2].map(|channel| {
                    ((u16::from(pixel[channel]) * alpha + base * (255 - alpha)) / 255) as u8
                })
            };
            let top = composite(&image.get_pixel(x, y).0);
            let bottom = composite(&image.get_pixel(x, (y + 1).min(image.height() - 1)).0);
            row.spans
                .push((row.text.len(), Decoration::role(Role::Pixel(top, bottom))));
            row.text.push('▀');
        }
        result.push(row);
    }
    if let Some(first_row) = result.get_mut(1) {
        first_row.raster = Some(RasterImage {
            area: Rect::new(
                Point::new(0, 0),
                Size::new(image.width() as u16, image.height().div_ceil(2) as u16),
            ),
            png,
        });
    }
    result.push(media_text(
        "Image preview · scaled to fit".into(),
        &context,
        Role::Quote,
    ));
    if let Some(first) = result.first_mut() {
        first.gap = context.gap;
    }
    result
}

#[derive(Clone)]
pub(super) struct DecodedImage {
    original: (u32, u32),
    pixels: RgbaImage,
    png: Arc<[u8]>,
}

pub(super) fn decode_image(source: &str) -> Result<DecodedImage, &'static str> {
    // Retain only four small thumbnails; reflow never repeatedly decodes the full image.
    static CACHE: OnceLock<Mutex<VecDeque<(String, DecodedImage)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Ok(cache) = cache.lock()
        && let Some((_, image)) = cache.iter().find(|(key, _)| key == source)
    {
        return Ok(image.clone());
    }
    let image = decode_image_uncached(source)?;
    if let Ok(mut cache) = cache.lock() {
        if cache.len() == 4 {
            cache.pop_front();
        }
        cache.push_back((source.to_owned(), image.clone()));
    }
    Ok(image)
}

fn decode_image_uncached(source: &str) -> Result<DecodedImage, &'static str> {
    let data = source
        .strip_prefix("data:image/png;base64,")
        .or_else(|| source.strip_prefix("data:image/jpeg;base64,"))
        .ok_or("Preview needs an embedded PNG or JPEG; link retained.")?;
    if data.len() > 1_398_104 {
        return Err("Image exceeds the 1 MiB preview limit.");
    }
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| "Image data is incomplete or invalid.")?;
    if bytes.len() > 1_048_576 {
        return Err("Image exceeds the 1 MiB preview limit.");
    }
    let original_bytes = bytes.clone();
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Image format is not recognized.")?;
    let is_png = reader.format() == Some(ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits.clone());
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| "Image cannot be decoded within preview limits.")?;
    // Preserve ImageReader::decode's output-buffer reservation before decoding.
    limits
        .reserve(decoder.total_bytes())
        .and_then(|()| decoder.set_limits(limits))
        .map_err(|_| "Image cannot be decoded within preview limits.")?;
    let orientation = decoder
        .orientation()
        .map_err(|_| "Image orientation cannot be read.")?;
    let mut image = DynamicImage::from_decoder(decoder)
        .map_err(|_| "Image cannot be decoded within preview limits.")?;
    image.apply_orientation(orientation);
    // A transformed PNG must carry corrected pixels without its old EXIF rotation.
    let png = if is_png && orientation == Orientation::NoTransforms {
        original_bytes
    } else {
        let mut encoded = Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, ImageFormat::Png)
            .map_err(|_| "Image conversion failed.")?;
        encoded.into_inner()
    };
    Ok(DecodedImage {
        png: png.into(),
        original: (image.width(), image.height()),
        pixels: image
            .thumbnail(image.width().min(64), image.height().min(40))
            .to_rgba8(),
    })
}
