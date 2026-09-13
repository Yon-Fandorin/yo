use super::*;

const BINDING: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../yo-core/src/model_service/tests/qwencloud-image-binding.json"
));

fn qwen_policy(tools: bool) -> ImageWirePolicy {
    let mut binding: Value = serde_json::from_str(BINDING).unwrap();
    if tools {
        binding["tool_capability_policy"] = json!("local-tools/v1");
    }
    ImageWirePolicy::admit(&CompleteModelBinding::from_durable_json(&binding.to_string()).unwrap())
        .unwrap()
        .unwrap()
}

fn tool_exposure(enabled: bool) -> RequestToolExposure {
    if enabled {
        RequestToolExposure::enabled(vec![yo_core::FunctionTool::new("read_file", "read one file", json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false})).unwrap()])
    } else {
        RequestToolExposure::disabled()
    }
}

// 모든 request 종류와 N=0에서 두 thinking 옵션과 명시적 auto를 유지하며 이미지 bytes만 추정에서
// 뺀다.
#[test]
fn thinking_and_tool_selection_survive_text_images_results_and_summary() {
    let user = ModelConnectorInputItem::Message {
        role: ModelConnectorInputRole::User,
        content: "next".into(),
        refusal: None,
    };
    let summary =
        ImageSummarySource::from_replay_groups(&[vec![ModelReplayItem::MultimodalUser {
            parts: parts(),
        }]])
        .unwrap();
    let scenarios = vec![
        vec![user.clone()],
        vec![ModelConnectorInputItem::MultimodalUser { parts: parts() }],
        vec![ModelConnectorInputItem::ImageSummarySource { source: summary }],
        vec![
            ModelConnectorInputItem::MultimodalUser { parts: parts() },
            ModelConnectorInputItem::FunctionCall {
                call_id: "c".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"marker"}"#.into(),
            },
            ModelConnectorInputItem::FunctionCallOutput {
                call_id: "c".into(),
                output: "literal marker".into(),
            },
            user,
        ],
    ];
    for enabled in [false, true] {
        let policy = qwen_policy(enabled);
        for input in &scenarios {
            let req =
                ModelConnectorRequest::new(input.clone(), tool_exposure(enabled), 131072, None)
                    .unwrap();
            let body = projected_body(&req, "qwen3.8-flash", Some(&policy), false).unwrap();
            let tokens = projected_body(&req, "qwen3.8-flash", Some(&policy), true).unwrap();
            assert_eq!(body["enable_thinking"], false);
            assert_eq!(body["preserve_thinking"], false);
            assert_eq!(body["max_tokens"], 131072);
            if enabled {
                assert_eq!(body["tool_choice"], "auto");
                assert_eq!(body["tools"][0]["function"]["name"], "read_file");
            } else {
                assert!(body.get("tools").is_none());
                assert!(body.get("tool_choice").is_none());
            }
            let mut image_free = body;
            for message in image_free["messages"].as_array_mut().unwrap() {
                if let Some(content) = message["content"].as_array_mut() {
                    content.retain(|part| part["type"] != "image_url");
                }
            }
            assert_eq!(tokens, image_free);
        }
    }
}

// 완전한 history에 같은 PNG가 250회 나타나면 모두 세고 251번째를 tokenization 전에 거절한다.
#[test]
fn complete_history_charges_every_png_occurrence() {
    let image = ModelInputPart::Image {
        snapshot: snapshot(),
    };
    let mut input = vec![
        ModelConnectorInputItem::MultimodalUser {
            parts: vec![image.clone(); 16]
        };
        15
    ];
    input.push(ModelConnectorInputItem::MultimodalUser {
        parts: vec![image.clone(); 10],
    });
    let accepted = request(input.clone());
    assert_eq!(accepted.image_count(), 250);
    for tokenization in [false, true] {
        projected_body(
            &accepted,
            "qwen3.8-flash",
            Some(&qwen_policy(false)),
            tokenization,
        )
        .unwrap();
    }
    if let ModelConnectorInputItem::MultimodalUser { parts } = input.last_mut().unwrap() {
        parts.push(image);
    }
    let excess = request(input);
    assert_eq!(excess.image_count(), 251);
    for tokenization in [false, true] {
        assert!(
            projected_body(
                &excess,
                "qwen3.8-flash",
                Some(&qwen_policy(false)),
                tokenization
            )
            .is_err()
        );
    }
}

// 유효한 history PNG 합계 16 MiB는 허용하되 첫 초과 byte는 image-free 추정에서도 거절한다.
#[test]
fn complete_png_bytes_are_checked_before_image_free_projection() {
    let item = |bytes| ModelConnectorInputItem::MultimodalUser {
        parts: vec![ModelInputPart::Image {
            snapshot: InputImageSnapshot::new(2048, 1023, uncompressed_png(bytes)).unwrap(),
        }],
    };
    let accepted = request(vec![item(8_388_608), item(8_388_608)]);
    let excess = request(vec![item(8_388_608), item(8_388_609)]);
    assert_eq!(
        accepted
            .input_images()
            .map(|image| image.png().len())
            .sum::<usize>(),
        16_777_216
    );
    assert_eq!(
        excess
            .input_images()
            .map(|image| image.png().len())
            .sum::<usize>(),
        16_777_217
    );
    for tokenization in [false, true] {
        projected_body(
            &accepted,
            "qwen3.8-flash",
            Some(&qwen_policy(false)),
            tokenization,
        )
        .unwrap();
        assert!(
            projected_body(
                &excess,
                "qwen3.8-flash",
                Some(&qwen_policy(false)),
                tokenization
            )
            .is_err()
        );
    }
}

// 최종 JSON 32 MiB를 정확히 허용하고 첫 초과 byte를 거절하며 text-only·추정 경계에도 적용한다.
#[test]
fn complete_encoded_json_limit_includes_text_and_request_options() {
    let policy = qwen_policy(false);
    let make = |content| {
        request(vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content,
            refusal: None,
        }])
    };
    let prefix = projected_body(&make("x".into()), "qwen3.8-flash", Some(&policy), false)
        .unwrap()
        .to_string()
        .len()
        - 1;
    let content = "x".repeat(33_554_432 - prefix);
    let accepted = make(content.clone());
    let excess = make(content + "x");
    for tokenization in [false, true] {
        let body = projected_body(&accepted, "qwen3.8-flash", Some(&policy), tokenization).unwrap();
        assert_eq!(serde_json::to_vec(&body).unwrap().len(), 33_554_432);
        assert!(projected_body(&excess, "qwen3.8-flash", Some(&policy), tokenization).is_err());
    }
}

// Qwen도 알려진 output cap과 request-local 도구 노출 정책을 Connector에서 다시 검사한다.
#[test]
fn rejects_incompatible_cap_tools_and_reasoning() {
    let input = vec![ModelConnectorInputItem::MultimodalUser { parts: parts() }];
    for cap in [None, Some(131073)] {
        let req =
            ModelConnectorRequest::new(input.clone(), tool_exposure(false), cap, None).unwrap();
        assert!(projected_body(&req, "qwen3.8-flash", Some(&qwen_policy(false)), true).is_err());
    }
    let req = ModelConnectorRequest::new(input.clone(), tool_exposure(true), 2048, None).unwrap();
    assert!(projected_body(&req, "qwen3.8-flash", Some(&qwen_policy(false)), false).is_err());
    let req = ModelConnectorRequest::new(
        input,
        tool_exposure(false),
        2048,
        Some(yo_core::ReasoningEffort::High),
    )
    .unwrap();
    assert!(projected_body(&req, "qwen3.8-flash", Some(&qwen_policy(false)), false).is_err());
}

fn uncompressed_png(total_bytes: usize) -> Vec<u8> {
    // 빈 저장 DEFLATE block과 허용된 빈 IDAT로 canonical PNG를 정확한 byte 경계에 맞춘다.
    let raw_len: usize = (2048 * 4 + 1) * 1023;
    let base_len = raw_len + 63 + 5 * raw_len.div_ceil(u16::MAX as usize);
    let padding = total_bytes - base_len;
    let empty_blocks = (0..12)
        .find(|blocks| padding >= blocks * 5 && (padding - blocks * 5).is_multiple_of(12))
        .unwrap();
    let empty_chunks = (padding - empty_blocks * 5) / 12;
    let mut zlib = vec![0x78, 0x01];
    for _ in 0..empty_blocks {
        zlib.extend_from_slice(&[0, 0, 0, 255, 255]);
    }
    let mut remaining = raw_len;
    while remaining > 0 {
        let block = remaining.min(u16::MAX as usize);
        remaining -= block;
        zlib.push(u8::from(remaining == 0));
        zlib.extend_from_slice(&(block as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(block as u16)).to_le_bytes());
        zlib.resize(zlib.len() + block, 0);
    }
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
    let mut header = 2048_u32.to_be_bytes().to_vec();
    header.extend_from_slice(&1023_u32.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &zlib);
    for _ in 0..empty_chunks {
        chunk(b"IDAT", &[]);
    }
    chunk(b"IEND", &[]);
    png
}
