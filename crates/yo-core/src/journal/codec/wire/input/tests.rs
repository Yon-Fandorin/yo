use super::*;
use crate::{InputImageSnapshot, ModelInputPart};

fn snapshot() -> InputImageSnapshot {
    serde_json::from_str(r#"{"profile":"yo.input-image-rgba8-triangle/v1","mime_type":"image/png","width":1,"height":1,"byte_length":70,"sha256":"sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}"#).unwrap()
}

fn input() -> UserInput {
    UserInput::new("[image]")
        .with_images(vec![InputImage::new(0..7, 1234, snapshot()).unwrap()])
        .unwrap()
}

fn encoded(input: &UserInput) -> String {
    serde_json::to_string(&WireUserInput::try_from(input).unwrap()).unwrap()
}

fn decode(value: &str) -> Result<UserInput, String> {
    let wire = serde_json::from_str::<WireUserInput>(value).map_err(|error| error.to_string())?;
    UserInput::try_from(wire).map_err(|error| error.to_string())
}

// v3 writer의 field 순서와 PNG·source count를 실제 입력 codec 왕복에서 보존한다.
#[test]
fn v3_canonical_wire_preserves_bytes_and_source_charge() {
    let original = input();
    let bytes = encoded(&original);
    let expected_snapshot = serde_json::to_string(&snapshot()).unwrap();
    assert_eq!(
        bytes,
        format!(
            r#"{{"profile":"yo.structured-input/v3","text":"[image]","references":[],"images":[{{"start":0,"end":7,"projection":"[image]","source_byte_length":1234,"snapshot":{expected_snapshot}}}]}}"#
        )
    );
    let recovered = decode(&bytes).unwrap();
    assert_eq!(recovered, original);
    assert_eq!(encoded(&recovered), bytes);
    assert_eq!(recovered.images()[0].snapshot().png(), snapshot().png());
    assert_eq!(recovered.images()[0].source_byte_length(), 1234);
    assert_eq!(
        recovered.model_parts(),
        vec![ModelInputPart::Image {
            snapshot: snapshot()
        }]
    );
}

// text-only v1의 기존 field·byte는 그대로 유지하고 image field를 끼운 downgrade는 거부한다.
#[test]
fn legacy_v1_wire_is_unchanged_and_does_not_accept_images() {
    assert_eq!(
        encoded(&UserInput::new("plain\r\n")),
        r#"{"profile":"yo.structured-input/v1","text":"plain\r\n","references":[]}"#
    );
    let bytes = encoded(&input());
    assert!(decode(&bytes.replace("yo.structured-input/v3", "yo.structured-input/v1")).is_err());
    assert!(decode(&bytes.replace("yo.structured-input/v3", "yo.structured-input/v2")).is_err());
}

// 필수 source evidence와 closed occurrence 필드의 누락·null·중복은 전체 입력을 거절한다.
#[test]
fn v3_rejects_missing_null_duplicate_and_unknown_fields() {
    let original: serde_json::Value = serde_json::from_str(&encoded(&input())).unwrap();
    for mutation in [
        "missing-images",
        "null-images",
        "empty-images",
        "missing-source",
        "null-source",
        "zero-source",
        "excess-source",
        "null-display",
        "null-filename",
        "unknown-image",
        "bad-digest",
        "bad-dimension",
    ] {
        let mut value = original.clone();
        match mutation {
            "missing-images" => {
                value.as_object_mut().unwrap().remove("images");
            },
            "null-images" => value["images"] = serde_json::Value::Null,
            "empty-images" => value["images"] = serde_json::json!([]),
            "missing-source" => {
                value["images"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("source_byte_length");
            },
            "null-source" => value["images"][0]["source_byte_length"] = serde_json::Value::Null,
            "zero-source" => value["images"][0]["source_byte_length"] = 0.into(),
            "excess-source" => {
                value["images"][0]["source_byte_length"] = (InputImage::MAX_SOURCE_BYTES + 1).into()
            },
            "null-display" => value["images"][0]["display"] = serde_json::Value::Null,
            "null-filename" => value["images"][0]["display"] = serde_json::json!({"filename":null}),
            "unknown-image" => value["images"][0]["path"] = "/tmp/source.png".into(),
            "bad-digest" => {
                value["images"][0]["snapshot"]["sha256"] =
                    format!("sha256:{}", "0".repeat(64)).into()
            },
            "bad-dimension" => value["images"][0]["snapshot"]["width"] = 2.into(),
            _ => unreachable!(),
        }
        assert!(decode(&value.to_string()).is_err(), "{mutation}");
    }
    let bytes = encoded(&input());
    for (needle, duplicate) in [
        ("\"images\":[", "\"images\":[],\"images\":["),
        (
            "\"source_byte_length\":1234",
            "\"source_byte_length\":1234,\"source_byte_length\":1234",
        ),
        ("\"width\":1", "\"width\":1,\"width\":1"),
    ] {
        assert!(decode(&bytes.replace(needle, duplicate)).is_err());
    }
}

// escaping·base64·framing을 포함한 정확한 16 MiB는 받고 한 encoded byte 초과는 거절한다.
#[test]
fn v3_complete_canonical_encoding_accepts_exact_limit_only() {
    let overhead = encoded(&input()).len();
    let remaining = InputImage::MAX_ENCODED_INPUT_BYTES - overhead;
    let text = format!("[image]{}", "x".repeat(remaining));
    let exact = UserInput::new(text.clone())
        .with_images(input().images().to_vec())
        .unwrap();
    assert_eq!(encoded(&exact).len(), InputImage::MAX_ENCODED_INPUT_BYTES);
    assert_eq!(decode(&encoded(&exact)).unwrap(), exact);
    assert!(
        UserInput::new(format!("{text}x"))
            .with_images(input().images().to_vec())
            .is_err()
    );
    // A quote consumes two encoded bytes even though it is one UTF-8 byte.
    assert!(
        UserInput::new(format!("{}\"", &text[..text.len() - 1]))
            .with_images(input().images().to_vec())
            .is_err()
    );
}
