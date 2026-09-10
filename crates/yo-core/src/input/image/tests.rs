use super::*;
use crate::{ModelInputPart, ResolvedSkill, SkillReference, SkillReferenceScope, UserInput};

fn snapshot() -> InputImageSnapshot {
    serde_json::from_str(r#"{"profile":"yo.input-image-rgba8-triangle/v1","mime_type":"image/png","width":1,"height":1,"byte_length":70,"sha256":"sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}"#).unwrap()
}

fn image(span: Range<usize>) -> InputImage {
    InputImage::new(span, 512, snapshot()).unwrap()
}

// 붙은 occurrence만 이미지를 만들고 한글·인접 이미지·표시용 메타데이터는 정확히 보존한다.
#[test]
fn projection_preserves_order_and_unannotated_markers() {
    let snapshot = snapshot();
    let input = UserInput::new("가[image][image] literal [image]")
        .with_images(vec![
            image(3..10)
                .with_display(InputImageDisplay {
                    filename: Some("original.jpg".into()),
                    ..Default::default()
                })
                .unwrap(),
            image(10..17),
        ])
        .unwrap();
    assert_eq!(
        input.model_parts(),
        vec![
            ModelInputPart::Text { text: "가".into() },
            ModelInputPart::Image {
                snapshot: snapshot.clone()
            },
            ModelInputPart::Image { snapshot },
            ModelInputPart::Text {
                text: " literal [image]".into()
            },
        ]
    );
    assert_eq!(input.images()[0].source_byte_length(), 512);
    assert_eq!(UserInput::new("[image]").model_input(), "[image]");
    assert!(UserInput::new("[image]").images().is_empty());
}

// 마지막 이미지 뒤에는 독립 trailer를 붙이고 마지막 text가 있으면 그 text에만 덧붙인다.
#[test]
fn skill_trailer_never_crosses_an_image() {
    let selected = SkillReference::new(
        "skill:review",
        "host:one",
        "/skills/review/SKILL.md",
        "review",
        SkillReferenceScope::User,
        1,
        "sha256:exact",
    );
    for suffix in ["", " after"] {
        let text = format!("$review before[image]{suffix}");
        let plain = UserInput::with_references(
            text.clone(),
            vec![InputReference::skill(0..7, selected.clone())],
        )
        .unwrap()
        .with_resolved_skill(ResolvedSkill::new(selected.clone(), "Frozen instructions").unwrap())
        .unwrap();
        let trailer = plain.model_input()[text.len()..].to_owned();
        let input = plain.with_images(vec![image(14..21)]).unwrap();
        assert_eq!(
            input.model_parts(),
            vec![
                ModelInputPart::Text {
                    text: "$review before".into()
                },
                ModelInputPart::Image {
                    snapshot: snapshot()
                },
                ModelInputPart::Text {
                    text: format!("{suffix}{trailer}")
                },
            ]
        );
    }
}

// 원본 압축 크기를 PNG 길이로 대체하지 않고 반복 occurrence마다 과금하며 첫 초과를 거절한다.
#[test]
fn source_and_occurrence_budgets_charge_each_image() {
    assert!(InputImage::new(0..7, 0, snapshot()).is_err());
    assert!(InputImage::new(0..7, InputImage::MAX_SOURCE_BYTES + 1, snapshot()).is_err());
    let two = UserInput::new("[image][image]")
        .with_images(vec![
            InputImage::new(0..7, InputImage::MAX_SOURCE_BYTES, snapshot()).unwrap(),
            InputImage::new(7..14, InputImage::MAX_SOURCE_BYTES, snapshot()).unwrap(),
        ])
        .unwrap();
    let mut inherited = two.images().to_vec();
    inherited.push(InputImage::new(14..21, 1, snapshot()).unwrap());
    assert_eq!(
        UserInput::new("[image]".repeat(3))
            .with_images(inherited)
            .unwrap_err(),
        UserInputError::ImageBudgetExceeded
    );
    for count in [16, 17] {
        let images = (0..count)
            .map(|index| image(index * 7..index * 7 + 7))
            .collect();
        assert_eq!(
            UserInput::new("[image]".repeat(count))
                .with_images(images)
                .is_ok(),
            count == 16
        );
    }
}

// 입력을 분할하는 16개 이미지는 최대 33개 순서 part를 만들고 원래 텍스트를 합치지 않는다.
#[test]
fn sixteen_images_preserve_all_thirty_three_parts() {
    let input = UserInput::new(format!("{}x", "x[image]".repeat(16)))
        .with_images(
            (0..16)
                .map(|index| image(index * 8 + 1..index * 8 + 8))
                .collect(),
        )
        .unwrap();
    assert_eq!(input.model_parts().len(), 33);
    ModelInputPart::validate_user_parts(&input.model_parts()).unwrap();
}

// 서로 겹치거나 순서가 바뀐 이미지와 reference span은 승인되지 않는다.
#[test]
fn invalid_spans_and_reference_overlap_reject_whole_input() {
    for images in [
        vec![image(1..8)],
        vec![image(7..14), image(0..7)],
        vec![image(0..7), image(0..7)],
    ] {
        assert!(
            UserInput::new("[image][image]")
                .with_images(images)
                .is_err()
        );
    }
    let input = UserInput::from_validated_persisted_v1(
        "[image]".into(),
        vec![InputReference::persisted_skill(
            0..7,
            "[image]".into(),
            SkillReference::new(
                "skill:one",
                "host",
                "/skill",
                "one",
                SkillReferenceScope::User,
                1,
                "revision",
            ),
        )],
    );
    assert!(input.with_images(vec![image(0..7)]).is_err());
}

// 표시 metadata는 경로·제어문자·초과 치수·비정규 digest를 수용하지 않는다.
#[test]
fn display_metadata_is_bounded_without_entering_projection() {
    for filename in [
        "",
        "../photo.png",
        "dir\\photo.png",
        "bad\nname",
        &"x".repeat(256),
    ] {
        assert!(
            image(0..7)
                .with_display(InputImageDisplay {
                    filename: Some(filename.into()),
                    ..Default::default()
                })
                .is_err()
        );
    }
    assert!(
        image(0..7)
            .with_display(InputImageDisplay {
                filename: Some("x".repeat(255)),
                source_width: Some(4096),
                source_height: Some(1024),
                ..Default::default()
            })
            .is_ok()
    );
    for display in [
        InputImageDisplay {
            source_width: Some(4097),
            ..Default::default()
        },
        InputImageDisplay {
            source_width: Some(4096),
            source_height: Some(1025),
            ..Default::default()
        },
        InputImageDisplay {
            source_height: Some(0),
            ..Default::default()
        },
        InputImageDisplay {
            source_mime_type: Some("image/gif".into()),
            ..Default::default()
        },
        InputImageDisplay {
            source_sha256: Some(format!("sha256:{}", "A".repeat(64))),
            ..Default::default()
        },
    ] {
        assert!(image(0..7).with_display(display).is_err());
    }
}

fn uncompressed_rgba_png(width: u32, height: u32) -> Vec<u8> {
    // Stored DEFLATE blocks encode a real all-transparent image without another image library.
    let raw_len = (width as usize * 4 + 1) * height as usize;
    let mut zlib = vec![0x78, 0x01];
    let mut remaining = raw_len;
    while remaining > 0 {
        let block = remaining.min(u16::MAX as usize);
        remaining -= block;
        zlib.push(u8::from(remaining == 0));
        zlib.extend_from_slice(&(block as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(block as u16)).to_le_bytes());
        zlib.resize(zlib.len() + block, 0);
    }
    // Adler-32 of all zero bytes has a=1 and b=the byte count modulo 65521.
    zlib.extend_from_slice(&(((raw_len % 65521) as u32) << 16 | 1).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        let mut crc = u32::MAX;
        for byte in kind.iter().chain(data) {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320_u32 & 0_u32.wrapping_sub(crc & 1));
            }
        }
        png.extend_from_slice(&(!crc).to_be_bytes());
    };
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &zlib);
    chunk(b"IEND", &[]);
    png
}

// 각각 유효한 PNG라도 반복 attachment의 합계가 9 MiB를 넘으면 원본 source budget과 별도로 거절한다.
#[test]
fn canonical_png_aggregate_charges_repeated_occurrences() {
    let png = uncompressed_rgba_png(2048, 1024);
    let snapshot = InputImageSnapshot::new(2048, 1024, png).unwrap();
    assert!(snapshot.png().len() < InputImageSnapshot::MAX_BYTES);
    assert!(snapshot.png().len() * 2 > InputImageSnapshot::MAX_BYTES);
    let one = InputImage::new(0..7, 1, snapshot.clone()).unwrap();
    assert!(
        UserInput::new("[image]")
            .with_images(vec![one.clone()])
            .is_ok()
    );
    let two = InputImage::new(7..14, 1, snapshot).unwrap();
    assert_eq!(
        UserInput::new("[image][image]")
            .with_images(vec![one, two])
            .unwrap_err(),
        UserInputError::ImageBudgetExceeded
    );
}
