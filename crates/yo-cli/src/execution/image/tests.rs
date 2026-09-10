use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use image::{ExtendedColorType, Rgba, codecs::jpeg::JpegEncoder};

use super::*;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "yo-image-prepare-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn source_png(width: u32, height: u32) -> Vec<u8> {
    let pixels = RgbaImage::from_fn(width, height, |x, y| Rgba([x as u8, y as u8, 123, 255]));
    encode(&pixels, MAX_SOURCE_BYTES).unwrap()
}

fn digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut digest = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        write!(digest, "{byte:02x}").unwrap();
    }
    digest
}

fn exif_rotation() -> Vec<u8> {
    // Little-endian TIFF: one SHORT orientation tag with value 6 (clockwise).
    vec![
        b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
    ]
}

// 확장자 대신 캡처한 내용으로 판별하고 원본을 보존하며 스냅샷과 썸네일을 생성한다.
#[test]
fn regular_file_prepares_exact_pixels_and_observed_source_evidence() {
    let directory = TestDirectory::new();
    let path = directory.0.join("synthetic.wrong-extension");
    let source = source_png(19, 11);
    fs::write(&path, &source).unwrap();
    let prepared = prepare(&path, &mut || false).unwrap();
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(prepared.source_byte_length(), source.len() as u64);
    assert_eq!(prepared.source_mime_type(), "image/png");
    assert_eq!(
        (prepared.source_width(), prepared.source_height()),
        (19, 11)
    );
    assert_eq!(prepared.source_sha256(), digest(&source));
    assert_eq!(
        prepared.snapshot().sha256(),
        digest(prepared.snapshot().png())
    );
    let expected = image::load_from_memory(&source).unwrap().to_rgba8();
    assert_eq!(
        image::load_from_memory(prepared.snapshot().png())
            .unwrap()
            .to_rgba8(),
        expected
    );
    assert_eq!(
        image::load_from_memory(prepared.thumbnail_png())
            .unwrap()
            .to_rgba8(),
        expected
    );
}

// PNG와 JPEG의 EXIF 회전을 적용하되 표시용 원본 크기는 헤더 값으로 유지한다.
#[test]
fn png_and_jpeg_orientation_is_applied_and_metadata_is_removed() {
    let pixels = RgbaImage::from_fn(3, 2, |x, y| {
        Rgba([(x * 70) as u8, (y * 180) as u8, 42, 255])
    });
    let mut png = Vec::new();
    let mut encoder = PngEncoder::new(&mut png);
    encoder.set_exif_metadata(exif_rotation()).unwrap();
    encoder
        .write_image(pixels.as_raw(), 3, 2, ExtendedColorType::Rgba8)
        .unwrap();
    let rgb = DynamicImage::ImageRgba8(pixels).to_rgb8();
    let mut jpeg = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, 95);
    encoder.set_exif_metadata(exif_rotation()).unwrap();
    encoder
        .write_image(rgb.as_raw(), 3, 2, ExtendedColorType::Rgb8)
        .unwrap();
    for (source, mime) in [(png, "image/png"), (jpeg, "image/jpeg")] {
        let original = image::load_from_memory(&source).unwrap().to_rgba8();
        let prepared = prepare_source(&source, &mut || false).unwrap();
        assert_eq!(prepared.source_mime_type(), mime);
        assert_eq!((prepared.source_width(), prepared.source_height()), (3, 2));
        assert_eq!(
            (prepared.snapshot().width(), prepared.snapshot().height()),
            (2, 3)
        );
        let actual = image::load_from_memory(prepared.snapshot().png())
            .unwrap()
            .to_rgba8();
        for y in 0..3 {
            for x in 0..2 {
                assert_eq!(actual.get_pixel(x, y), original.get_pixel(y, 1 - x));
            }
        }
        let mut decoder = PngDecoder::new(Cursor::new(prepared.snapshot().png())).unwrap();
        assert_eq!(decoder.exif_metadata().unwrap(), None);
        assert_eq!(decoder.icc_profile().unwrap(), None);
        assert_eq!(
            image::load_from_memory(prepared.thumbnail_png())
                .unwrap()
                .to_rgba8(),
            actual
        );
    }
}

// IDAT 뒤에 있는 PNG EXIF도 회전시키며 잘못된 EXIF 체크섬은 거부한다.
#[test]
fn png_orientation_after_image_data_is_honored() {
    let png = source_png(3, 2);
    let mut late_exif = png[..png.len() - 12].to_vec();
    append_chunk(&mut late_exif, b"eXIf", &exif_rotation());
    late_exif.extend_from_slice(&png[png.len() - 12..]);
    let prepared = prepare_source(&late_exif, &mut || false).unwrap();
    assert_eq!(
        (prepared.snapshot().width(), prepared.snapshot().height()),
        (2, 3)
    );
    let original = image::load_from_memory(&png).unwrap().to_rgba8();
    let actual = image::load_from_memory(prepared.snapshot().png())
        .unwrap()
        .to_rgba8();
    assert_eq!(actual.get_pixel(0, 0), original.get_pixel(0, 1));
    let checksum_byte = late_exif.len() - 13;
    late_exif[checksum_byte] ^= 1;
    assert!(prepare_source(&late_exif, &mut || false).is_err());
}

// 16비트 PNG는 고정된 image 0.25.10의 반올림 양자화로 RGBA8이 된다.
#[test]
fn sixteen_bit_png_uses_pinned_rgba8_quantization() {
    let samples = [0_u16, 128, 129, 256, 32_768, 65_535];
    let raw: Vec<u8> = samples.into_iter().flat_map(u16::to_ne_bytes).collect();
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(&raw, 6, 1, ExtendedColorType::L16)
        .unwrap();
    let prepared = prepare_source(&png, &mut || false).unwrap();
    let actual = image::load_from_memory(prepared.snapshot().png())
        .unwrap()
        .to_rgba8();
    let expected = [0_u8, 0, 1, 1, 128, 255];
    for (pixel, value) in actual.pixels().zip(expected) {
        assert_eq!(pixel.0, [value, value, value, 255]);
    }
}

// 정규화는 픽셀 수와 변 길이를 함께 제한하고 썸네일은 정규화된 픽셀에서 만든다.
#[test]
fn scaling_respects_both_limits_and_thumbnail_bound() {
    for (width, height, expected) in [
        (2048, 2048, (1448, 1448)),
        (4096, 1024, (2048, 512)),
        (1, 1, (1, 1)),
    ] {
        let source = source_png(width, height);
        let prepared = prepare_source(&source, &mut || false).unwrap();
        assert_eq!(
            (prepared.snapshot().width(), prepared.snapshot().height()),
            expected
        );
        assert!(prepared.snapshot().png().len() <= InputImageSnapshot::MAX_BYTES);
        let thumbnail = image::load_from_memory(prepared.thumbnail_png()).unwrap();
        assert!(thumbnail.width() <= 256 && thumbnail.height() <= 256);
        assert!(thumbnail.width() <= expected.0 && thumbnail.height() <= expected.1);
        assert!(prepared.thumbnail_png().len() <= MAX_THUMBNAIL_BYTES);
    }
}

// 원본 크기의 첫 초과 변과 픽셀 수는 디코딩 결과가 게시되기 전에 거부한다.
#[test]
fn source_dimensions_accept_boundary_and_reject_first_excess() {
    assert!(check_dimensions(4096, 1024, MAX_SOURCE_SIDE, MAX_SOURCE_PIXELS).is_ok());
    assert!(check_dimensions(4097, 1, MAX_SOURCE_SIDE, MAX_SOURCE_PIXELS).is_err());
    assert!(check_dimensions(2048, 2049, MAX_SOURCE_SIDE, MAX_SOURCE_PIXELS).is_err());
    assert!(check_dimensions(0, 1, MAX_SOURCE_SIDE, MAX_SOURCE_PIXELS).is_err());
    for (width, height) in [(4097, 1), (2048, 2049)] {
        assert!(prepare_source(&source_png(width, height), &mut || false).is_err());
    }
}

// 압축 원본은 경계까지 허용하며 읽기는 첫 초과 바이트에서 멈춘다.
#[test]
fn source_reader_stops_at_first_excess_byte() {
    let mut exact = Cursor::new(vec![42; MAX_SOURCE_BYTES]);
    assert_eq!(
        read_source(&mut exact, &mut || false).unwrap().len(),
        MAX_SOURCE_BYTES
    );
    let mut excess = Cursor::new(vec![42; MAX_SOURCE_BYTES + 17]);
    assert!(read_source(&mut excess, &mut || false).is_err());
    assert_eq!(excess.position(), (MAX_SOURCE_BYTES + 1) as u64);
    let directory = TestDirectory::new();
    let path = directory.0.join("large.png");
    fs::write(&path, vec![0; MAX_SOURCE_BYTES + 1]).unwrap();
    assert!(prepare(&path, &mut || false).is_err());
    assert!(prepare(&directory.0, &mut || false).is_err());
}

// PNG 출력의 첫 초과 쓰기는 버퍼를 늘리지 않으며 부분 인코딩은 반환하지 않는다.
#[test]
fn bounded_writer_rejects_excess_before_extending_and_encoder_discards_partial() {
    for limit in [MAX_THUMBNAIL_BYTES, InputImageSnapshot::MAX_BYTES] {
        let mut writer = BoundedWriter {
            bytes: Vec::new(),
            limit,
            exceeded: false,
        };
        writer.write_all(&vec![0; limit]).unwrap();
        assert!(writer.write_all(&[1]).is_err());
        assert!(writer.exceeded);
        assert_eq!(writer.bytes.len(), limit);
    }
    let pixels = RgbaImage::new(2, 2);
    let png = encode(&pixels, 1024).unwrap();
    assert_eq!(encode(&pixels, png.len()).unwrap(), png);
    // 마지막 IEND 쓰기의 오류도 최종 PNG 인코딩 실패로 전달한다.
    assert!(encode(&pixels, png.len() - 1).is_err());
}

// 읽기 전과 읽는 중, 각 준비 단계의 취소는 준비 결과를 반환하지 않는다.
#[test]
fn cancellation_rejects_before_read_and_at_every_preparation_boundary() {
    let mut reader = Cursor::new(vec![0; 128 * 1024]);
    assert!(read_source(&mut reader, &mut || true).is_err());
    assert_eq!(reader.position(), 0);
    let mut calls = 0;
    assert!(
        read_source(&mut reader, &mut || {
            calls += 1;
            calls == 3
        })
        .is_err()
    );
    assert_eq!(reader.position(), 64 * 1024);
    let source = source_png(3, 2);
    let mut boundaries = 0;
    prepare_source(&source, &mut || {
        boundaries += 1;
        false
    })
    .unwrap();
    for cancelled_at in 1..=boundaries {
        let mut calls = 0;
        assert!(
            prepare_source(&source, &mut || {
                calls += 1;
                calls == cancelled_at
            })
            .is_err()
        );
    }
}

// 정적 PNG 이외 형식과 APNG 및 손상된 입력은 준비할 수 없다.
#[test]
fn rejects_animation_other_formats_and_truncated_images() {
    let png = source_png(2, 2);
    let mut apng = png[..33].to_vec();
    append_chunk(&mut apng, b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]);
    let mut frame = Vec::new();
    for value in [0_u32, 2, 2, 0, 0] {
        frame.extend_from_slice(&value.to_be_bytes());
    }
    frame.extend_from_slice(&[0, 1, 0, 10, 0, 0]);
    append_chunk(&mut apng, b"fcTL", &frame);
    apng.extend_from_slice(&png[33..]);
    assert!(
        PngDecoder::new(Cursor::new(&apng))
            .unwrap()
            .is_apng()
            .unwrap()
    );
    for source in [
        &apng[..],
        &b"GIF89a\x01\0\x01\0"[..],
        &b"RIFF\0\0\0\0WEBP"[..],
        &png[..png.len() / 2],
        &b""[..],
    ] {
        assert!(prepare_source(source, &mut || false).is_err());
    }
}

fn append_chunk(png: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    png.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    let crc_start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(payload);
    let crc = png_crc32(&png[crc_start..]);
    png.extend_from_slice(&crc.to_be_bytes());
}
