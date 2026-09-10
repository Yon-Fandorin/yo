use serde_json::{Value, json};

use super::*;

const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
const SHA256: &str = "sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5";

fn png() -> Vec<u8> {
    STANDARD.decode(PNG).unwrap()
}

fn wire() -> Value {
    json!({
        "profile": "yo.input-image-rgba8-triangle/v1",
        "mime_type": "image/png",
        "width": 1,
        "height": 1,
        "byte_length": 70,
        "sha256": SHA256,
        "data_base64": PNG,
    })
}

// 실제 1픽셀 RGBA PNG가 고정된 wire 순서와 바이트로 왕복하며 복제는 원본 바이트를 공유한다.
#[test]
fn canonical_wire_preserves_exact_png_and_shares_immutable_storage() {
    let snapshot: InputImageSnapshot = serde_json::from_value(wire()).unwrap();
    assert_eq!(snapshot.png(), png());
    assert_eq!(snapshot.sha256(), SHA256);
    let expected = format!(
        "{{\"profile\":\"yo.input-image-rgba8-triangle/v1\",\"mime_type\":\"image/png\",\"width\":1,\"height\":1,\"byte_length\":70,\"sha256\":\"{SHA256}\",\"data_base64\":\"{PNG}\"}}"
    );
    assert_eq!(serde_json::to_string(&snapshot).unwrap(), expected);
    assert_eq!(
        serde_json::from_str::<InputImageSnapshot>(&expected).unwrap(),
        snapshot
    );
    let clone = snapshot.clone();
    assert!(Arc::ptr_eq(&snapshot.png, &clone.png));
    assert!(!format!("{snapshot:?}").contains(PNG));
}

// 저장된 이미지 envelope는 필드 누락·null·중복·추가를 조용히 보정하지 않고 전체를 거절한다.
#[test]
fn snapshot_wire_is_closed_and_every_field_is_required() {
    for field in [
        "profile",
        "mime_type",
        "width",
        "height",
        "byte_length",
        "sha256",
        "data_base64",
    ] {
        let mut absent = wire();
        absent.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<InputImageSnapshot>(absent).is_err(),
            "missing {field}"
        );
        let mut null = wire();
        null[field] = Value::Null;
        assert!(
            serde_json::from_value::<InputImageSnapshot>(null).is_err(),
            "null {field}"
        );
        let encoded = serde_json::to_string(&wire()).unwrap();
        let duplicate = format!("{{\"{field}\":{},{}", wire()[field], &encoded[1..]);
        assert!(
            serde_json::from_str::<InputImageSnapshot>(&duplicate).is_err(),
            "duplicate {field}"
        );
    }
    let mut unknown = wire();
    unknown["path"] = json!("source.png");
    assert!(serde_json::from_value::<InputImageSnapshot>(unknown).is_err());
}

// 동일 바이트의 다른 프로필·MIME·크기·해시 또는 헤더와 다른 치수는 복구 입력이 될 수 없다.
#[test]
fn snapshot_rejects_identity_mismatch_and_invalid_numeric_domains() {
    for (field, values) in [
        ("profile", vec![json!("yo.input-image-rgba8-triangle/v2")]),
        ("mime_type", vec![json!("image/jpeg")]),
        (
            "sha256",
            vec![json!(SHA256.to_uppercase()), json!("sha256:00")],
        ),
        (
            "byte_length",
            vec![json!(0), json!(69), json!(71), json!(-1), json!(70.0)],
        ),
        (
            "width",
            vec![json!(0), json!(2), json!(2049), json!(-1), json!(1.0)],
        ),
        (
            "height",
            vec![json!(0), json!(2), json!(2049), json!(-1), json!(1.0)],
        ),
    ] {
        for value in values {
            let mut changed = wire();
            changed[field] = value;
            assert!(
                serde_json::from_value::<InputImageSnapshot>(changed).is_err(),
                "{field}"
            );
        }
    }
    assert_eq!(
        InputImageSnapshot::new(2048, 2048, png()),
        Err(InputImageSnapshotError::Dimensions)
    );
}

// base64의 공백·생략된 padding·사용하지 않는 하위 비트와 첫 초과 문자를 허용하지 않는다.
#[test]
fn snapshot_rejects_noncanonical_base64_before_accepting_bytes() {
    for value in [
        format!(" {PNG}"),
        PNG.trim_end_matches('=').to_owned(),
        format!("{}h==", &PNG[..PNG.len() - 3]),
        "A".repeat(InputImageSnapshot::MAX_BYTES.div_ceil(3) * 4 + 1),
    ] {
        let mut changed = wire();
        changed["data_base64"] = json!(value);
        assert!(serde_json::from_value::<InputImageSnapshot>(changed).is_err());
    }
}

// 바이트 첫 초과는 컨테이너 파싱 전 거절하며 PNG 손상·잘림·후행 데이터도 거절한다.
#[test]
fn snapshot_checks_first_excess_byte_and_png_container_integrity() {
    assert_eq!(
        InputImageSnapshot::new(1, 1, vec![0; InputImageSnapshot::MAX_BYTES + 1]),
        Err(InputImageSnapshotError::ByteLength)
    );
    for end in 0..png().len() {
        assert!(InputImageSnapshot::new(1, 1, png()[..end].to_vec()).is_err());
    }
    let mut changed = png();
    changed[45] ^= 1;
    assert_eq!(
        InputImageSnapshot::new(1, 1, changed),
        Err(InputImageSnapshotError::Encoding)
    );
    let mut trailing = png();
    trailing.push(0);
    assert!(InputImageSnapshot::new(1, 1, trailing).is_err());
}

fn canonical_with_idat(chunks: &[&[u8]]) -> Vec<u8> {
    let mut value = png()[..33].to_vec();
    for bytes in chunks {
        value.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        value.extend_from_slice(b"IDAT");
        value.extend_from_slice(bytes);
        let start = value.len() - bytes.len() - 4;
        value.extend_from_slice(&crc32fast::hash(&value[start..]).to_be_bytes());
    }
    value.extend_from_slice(&png()[png().len() - 12..]);
    value
}

fn compressed_pixels(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;

    use flate2::{Compression, write::ZlibEncoder};
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

// 여러 IDAT와 빈 조각은 보존하되 CRC가 맞아도 잘못된 zlib·scanline은 durable 이미지가 될 수 없다.
#[test]
fn canonical_pixel_stream_requires_exact_rows_checksum_and_end() {
    let valid = compressed_pixels(&[0, 255, 0, 0, 255]);
    let mut pieces = vec![&[][..]];
    pieces.extend(valid.chunks(1));
    pieces.push(&[]);
    InputImageSnapshot::new(1, 1, canonical_with_idat(&pieces)).unwrap();
    let mut bad_checksum = valid.clone();
    *bad_checksum.last_mut().unwrap() ^= 1;
    let streams = [
        vec![1, 2, 3],
        compressed_pixels(&[0, 255, 0, 0]),
        compressed_pixels(&[0, 255, 0, 0, 255, 0]),
        compressed_pixels(&[5, 255, 0, 0, 255]),
        bad_checksum,
        valid[..valid.len() - 1].to_vec(),
        [valid.as_slice(), valid.as_slice()].concat(),
        [valid.as_slice(), &[0]].concat(),
    ];
    for (index, stream) in streams.iter().enumerate() {
        assert!(
            InputImageSnapshot::new(1, 1, canonical_with_idat(&[stream])).is_err(),
            "invalid stream {index}"
        );
    }
}
