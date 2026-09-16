use super::support::*;

// 실제 PNG의 서로 다른 픽셀 색과 크기가 cell preview까지 유지되고, 실패한 이미지의
// base64 문자열은 본문에 쏟아지지 않는다. decode 크기 제한은 원본에 적용된다.
#[test]
fn embedded_images_decode_bound_dimensions_and_hide_invalid_payloads() {
    use ::image::{ImageFormat, Rgb, RgbImage};
    let pixels = RgbImage::from_fn(4, 2, |x, _| {
        if x < 2 {
            Rgb([255, 0, 0])
        } else {
            Rgb([0, 0, 255])
        }
    });
    let mut bytes = Cursor::new(Vec::new());
    pixels.write_to(&mut bytes, ImageFormat::Png).unwrap();
    let source = format!(
        "![Color check](data:image/png;base64,{})",
        STANDARD.encode(bytes.into_inner())
    );
    let rendered = prepare(&source, NonZeroU16::new(40).unwrap()).unwrap();
    assert!(
        rendered
            .glyphs
            .iter()
            .any(|g| g.decoration.role == Role::Pixel([255, 0, 0], [255, 0, 0]))
    );
    assert!(
        rendered
            .glyphs
            .iter()
            .any(|g| g.decoration.role == Role::Pixel([0, 0, 255], [0, 0, 255]))
    );
    assert!(rows(&rendered).join("\n").contains("4 × 2"));
    assert_eq!(rendered.rasters[0].area.size, Size::new(4, 1));
    let partial = prepare(
        "![Photo](data:image/png;base64,VGhpcy1pcy1ub3QtcG5n",
        NonZeroU16::new(40).unwrap(),
    )
    .unwrap();
    assert!(!rows(&partial).join("\n").contains("VGhpcy"));
    let large = RgbImage::new(2049, 1);
    let mut bytes = Cursor::new(Vec::new());
    large.write_to(&mut bytes, ImageFormat::Png).unwrap();
    assert!(
        decode_image(&format!(
            "data:image/png;base64,{}",
            STANDARD.encode(bytes.into_inner())
        ))
        .is_err()
    );
}

// 색상 미지원과 ASCII 설정에서도 이미지가 빈 사각형이 되지 않고 명암으로 남는다.
#[test]
fn media_fallback_uses_ascii_density_and_indexed_colors() {
    let plain = MarkdownStyles::plain(Style::default());
    let dark = Decoration::role(Role::Pixel([0, 0, 0], [0, 0, 0]));
    let light = Decoration::role(Role::Pixel([255, 255, 255], [255, 255, 255]));
    assert_eq!(
        plain
            .display_glyph(dark, Grapheme::try_from("▀").unwrap())
            .as_str(),
        " "
    );
    assert_eq!(
        plain
            .display_glyph(light, Grapheme::try_from("▀").unwrap())
            .as_str(),
        "@"
    );
    assert_eq!(
        plain.resolve(light, Style::default()).background,
        Color::Default
    );
    assert!(matches!(
        pixel_color([123, 45, 67], Color::Indexed(1)),
        Color::Indexed(_)
    ));
    let ascii = MarkdownStyles {
        rich_media: false,
        ..plain
    };
    assert_eq!(
        ascii
            .display_glyph(
                Decoration::role(Role::Chart),
                Grapheme::try_from("━").unwrap()
            )
            .as_str(),
        "="
    );
}

// 원본 PNG는 썸네일과 별도로 유지되어 terminal 전송 시 해상도를 잃지 않는다.
#[test]
fn native_image_keeps_original_png_and_bounded_cell_placement() {
    use ::image::{ImageFormat, RgbImage};
    let image = RgbImage::new(120, 60);
    let mut encoded = Cursor::new(Vec::new());
    image.write_to(&mut encoded, ImageFormat::Png).unwrap();
    let bytes = encoded.into_inner();
    let markdown = format!(
        "![Original](data:image/png;base64,{})",
        STANDARD.encode(&bytes)
    );
    let prepared = prepare(&markdown, NonZeroU16::new(40).unwrap()).unwrap();
    assert_eq!(prepared.rasters.len(), 1);
    let raster = &prepared.rasters[0];
    assert_eq!(&*raster.png, bytes.as_slice());
    assert!(raster.area.size.width <= 40);
    assert!(raster.area.size.height <= 20);
    assert!(raster.area.origin.y + raster.area.size.height <= prepared.height);
}

// 표시를 끄면 잘못된 데이터도 디코딩 오류나 원문 URL 대신 대체 설명만 남긴다.
#[test]
fn disabled_images_skip_decode_and_preserve_alt_text() {
    let prepared = prepare_with_images(
        "![설명](data:image/png;base64,INVALID)",
        NonZeroU16::new(40).unwrap(),
        false,
        NonZeroU16::new(8).unwrap(),
    )
    .unwrap();
    let text = rows(&prepared).join("\n");
    assert!(text.contains("설명"));
    assert!(text.contains("Image display disabled"));
    assert!(!text.contains("INVALID"));
    assert!(!text.contains("base64"));
    assert!(prepared.rasters.is_empty());
}

// Mermaid 흐름도·시퀀스는 실제 연결선을 만들고 좁은 폭·비활성·잘못된 문법은 원문을 보존한다.
#[test]
fn mermaid_diagrams_render_and_fall_back_without_clipping_source() {
    for source in [
        "graph TD; A[Build] --> B[Test]",
        "sequenceDiagram\nAlice->>Bob: Hello",
    ] {
        let fenced = format!("```mermaid\n{source}\n```");
        let view = prepare(&fenced, NonZeroU16::new(100).unwrap()).unwrap();
        let text = rows(&view).join("\n");
        assert!(!text.contains("shown as source"), "{text}");
        assert!(text.contains('│') || text.contains('─'), "{text}");
        assert!(!text.contains("graph TD"), "{text}");
        let disabled = prepare_with_media(
            &fenced,
            NonZeroU16::new(100).unwrap(),
            true,
            NonZeroU16::new(64).unwrap(),
            false,
        )
        .unwrap();
        let text = rows(&disabled).join("\n");
        assert!(
            text.split_whitespace()
                .collect::<String>()
                .contains(&source.split_whitespace().collect::<String>()),
            "{text}"
        );
        let narrow = prepare(&fenced, NonZeroU16::new(12).unwrap()).unwrap();
        let text = rows(&narrow).join("\n");
        let joined = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        assert!(joined.contains("Diagramshownassource"), "{text}");
        assert!(
            joined.contains(
                &source
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
            ),
            "{text}"
        );
    }
    let invalid = prepare(
        "```mermaid\nnot a diagram\n```",
        NonZeroU16::new(80).unwrap(),
    )
    .unwrap();
    assert!(rows(&invalid).join("\n").contains("not a diagram"));
}

// 렌더링 전에 크기 상한을 적용하고 첫 초과 문장은 잘리지 않은 코드 원문으로 남긴다.
#[test]
fn mermaid_diagram_limits_preserve_the_first_excess_statement() {
    let source = format!(
        "graph TD;{}",
        (0..64)
            .map(|n| format!("N{n}-->N{};", n + 1))
            .collect::<String>()
    );
    let view = prepare(
        &format!("```mermaid\n{source}\n```"),
        NonZeroU16::new(80).unwrap(),
    )
    .unwrap();
    let text = rows(&view).join("\n");
    assert!(text.contains("diagram exceeds preview limits"), "{text}");
    assert!(
        text.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .contains("N63-->N64;"),
        "{text}"
    );
}

// PNG/JPEG EXIF의 회전·반전과 잘못된 값을 원본 픽셀로 검증하고 셀·PNG 방향을 일치시킨다.
#[test]
fn image_orientation_is_applied_to_pixels_dimensions_and_native_png() {
    use ::image::{
        ExtendedColorType, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Rgb, RgbImage,
        codecs::{jpeg::JpegEncoder, png::PngEncoder},
        load_from_memory,
        metadata::Orientation,
    };
    let pixels = RgbImage::from_fn(3, 2, |x, y| Rgb([(x * 80) as u8, (y * 180) as u8, 70]));
    for format in [ImageFormat::Jpeg, ImageFormat::Png] {
        let mut bytes = Cursor::new(Vec::new());
        pixels.write_to(&mut bytes, format).unwrap();
        let bytes = bytes.into_inner();
        let original = load_from_memory(&bytes).unwrap().to_rgb8();
        for orientation in 0_u16..=9 {
            let mut exif = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
            exif.extend_from_slice(&orientation.to_le_bytes());
            exif.extend_from_slice(&[0; 6]);
            let mut encoded = Vec::new();
            let mime = if format == ImageFormat::Png {
                let mut encoder = PngEncoder::new(&mut encoded);
                encoder.set_exif_metadata(exif).unwrap();
                encoder
                    .write_image(pixels.as_raw(), 3, 2, ExtendedColorType::Rgb8)
                    .unwrap();
                "png"
            } else {
                let mut encoder = JpegEncoder::new(&mut encoded);
                encoder.set_exif_metadata(exif).unwrap();
                encoder.encode_image(&pixels).unwrap();
                "jpeg"
            };
            let source = format!(
                "![Oriented](data:image/{mime};base64,{})",
                STANDARD.encode(&encoded)
            );
            for width in [40, 2, 40] {
                let rendered = prepare(&source, NonZeroU16::new(width).unwrap()).unwrap();
                let raster = rendered.rasters.first().unwrap();
                let corrected = load_from_memory(&raster.png).unwrap().to_rgb8();
                if (2..=8).contains(&orientation) {
                    let mut decoder = ImageReader::with_format(
                        Cursor::new(raster.png.as_ref()),
                        ImageFormat::Png,
                    )
                    .into_decoder()
                    .unwrap();
                    assert_eq!(decoder.orientation().unwrap(), Orientation::NoTransforms);
                } else if format == ImageFormat::Png {
                    assert_eq!(raster.png.as_ref(), encoded.as_slice());
                }
                let swapped = (5..=8).contains(&orientation);
                assert_eq!(
                    corrected.dimensions(),
                    if swapped { (2, 3) } else { (3, 2) }
                );
                for (x, y, pixel) in corrected.enumerate_pixels() {
                    let (sx, sy) = match orientation {
                        2 => (2 - x, y),
                        3 => (2 - x, 1 - y),
                        4 => (x, 1 - y),
                        5 => (y, x),
                        6 => (y, 1 - x),
                        7 => (2 - y, 1 - x),
                        8 => (2 - y, x),
                        _ => (x, y),
                    };
                    assert_eq!(
                        pixel,
                        original.get_pixel(sx, sy),
                        "{format:?} orientation {orientation} at {x},{y}"
                    );
                }
                if width == 40 {
                    let pairs = rendered
                        .glyphs
                        .iter()
                        .filter_map(|glyph| match glyph.decoration.role {
                            Role::Pixel(top, bottom) => Some((top, bottom)),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    let expected = (0..corrected.height())
                        .step_by(2)
                        .flat_map(|y| (0..corrected.width()).map(move |x| (x, y)))
                        .map(|(x, y)| {
                            (
                                corrected.get_pixel(x, y).0,
                                corrected
                                    .get_pixel(x, (y + 1).min(corrected.height() - 1))
                                    .0,
                            )
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(pairs, expected);
                    assert!(rows(&rendered).join(" ").contains(if swapped {
                        "2 × 3"
                    } else {
                        "3 × 2"
                    }));
                }
            }
        }
    }
}
