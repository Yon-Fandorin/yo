use serde_json::{Value, json};

use super::super::{
    ApiDialect, BindingProfileDigest, BindingProfileSchema, BindingProfileV1, ConnectorId,
    NormalizedEndpoint,
};

#[derive(Clone)]
struct ProfileFixture {
    endpoint: String,
    dialect: ApiDialect,
    tokenizer_profile: String,
    context_limit: u64,
    output_limit: u64,
    reasoning_parameters: Value,
    optional_request_parameters: Value,
    tool_capability_policy: String,
    verification_profile: String,
}

impl Default for ProfileFixture {
    fn default() -> Self {
        Self {
            endpoint: "https://EXAMPLE.com:443/v1///".to_owned(),
            dialect: ApiDialect::OpenAiResponses,
            tokenizer_profile: "cl100k_base/v1".to_owned(),
            context_limit: 128_000,
            output_limit: 16_384,
            reasoning_parameters: json!({"effort": "medium"}),
            optional_request_parameters: json!({"temperature": 0.0}),
            tool_capability_policy: "tools.none/v1".to_owned(),
            verification_profile: "semantic-terminal/v2".to_owned(),
        }
    }
}

impl ProfileFixture {
    fn build(&self) -> BindingProfileV1 {
        BindingProfileV1::new(
            NormalizedEndpoint::parse(&self.endpoint).unwrap(),
            self.dialect,
            ConnectorId::for_dialect(self.dialect),
            &self.tokenizer_profile,
            self.context_limit,
            self.output_limit,
            self.reasoning_parameters.clone(),
            self.optional_request_parameters.clone(),
            &self.tool_capability_policy,
            &self.verification_profile,
        )
        .unwrap()
    }
}

fn profile_with_parameters(
    reasoning_parameters: Value,
    optional_request_parameters: Value,
) -> BindingProfileV1 {
    ProfileFixture {
        reasoning_parameters,
        optional_request_parameters,
        ..ProfileFixture::default()
    }
    .build()
}

fn framed_fields(bytes: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut remaining = bytes;
    let mut fields = Vec::new();
    while !remaining.is_empty() {
        let name_length = u64::from_be_bytes(remaining[..8].try_into().unwrap()) as usize;
        remaining = &remaining[8..];
        let name = &remaining[..name_length];
        remaining = &remaining[name_length..];
        let value_length = u64::from_be_bytes(remaining[..8].try_into().unwrap()) as usize;
        remaining = &remaining[8..];
        let value = &remaining[..value_length];
        remaining = &remaining[value_length..];
        fields.push((name, value));
    }
    fields
}

fn framed_value<'a>(profile: &'a BindingProfileV1, field: &[u8]) -> &'a [u8] {
    let domain_length = b"yo.binding-profile/v1\0".len();
    framed_fields(&profile.canonical_bytes()[domain_length..])
        .into_iter()
        .find_map(|(name, value)| (name == field).then_some(value))
        .unwrap()
}

// v1 profile은 고정 domain 뒤에 정확한 열 개 field를 계약 순서와 u64be 길이로 넣고,
// endpoint·limit·RFC 8785 JSON의 실제 value byte 전체에서 재현 가능한 digest를 만든다.
#[test]
fn canonical_profile_uses_the_exact_v1_field_framing_and_digest() {
    let profile = profile_with_parameters(
        json!({"z": 3, "a": [true, null], "number": 333_333_333.333_333_3_f64}),
        json!({"temperature": 0.0, "metadata": {"b": 2, "a": 1}}),
    );
    let domain = b"yo.binding-profile/v1\0";

    assert_eq!(profile.schema(), BindingProfileSchema::V1);
    assert_eq!(BindingProfileSchema::V1.as_str(), "yo.binding-profile/v1");
    assert!(profile.canonical_bytes().starts_with(domain));
    assert_eq!(
        framed_fields(&profile.canonical_bytes()[domain.len()..]),
        vec![
            (
                b"normalized_endpoint".as_slice(),
                b"https://example.com/v1".as_slice()
            ),
            (b"api_dialect".as_slice(), b"openai-responses".as_slice()),
            (
                b"resolved_connector_id".as_slice(),
                b"openai-responses".as_slice()
            ),
            (
                b"tokenizer_profile".as_slice(),
                b"cl100k_base/v1".as_slice()
            ),
            (b"context_limit".as_slice(), b"128000".as_slice()),
            (b"output_limit".as_slice(), b"16384".as_slice()),
            (
                b"reasoning_parameters".as_slice(),
                br#"{"a":[true,null],"number":333333333.3333333,"z":3}"#.as_slice(),
            ),
            (
                b"optional_request_parameters".as_slice(),
                br#"{"metadata":{"a":1,"b":2},"temperature":0}"#.as_slice(),
            ),
            (
                b"tool_capability_policy".as_slice(),
                b"tools.none/v1".as_slice()
            ),
            (
                b"verification_profile".as_slice(),
                b"semantic-terminal/v2".as_slice()
            ),
        ]
    );
    assert_eq!(
        profile.digest().as_str(),
        "sha256:da1c11b54f93c9ba4e9b198e50fe429dd54d2f758ff1b6bed9fce4709bc66dc0"
    );
}

// JSON object의 입력 순서가 달라도 RFC 8785 byte가 같으면 profile digest가 같고,
// 구조화하지 않는 versioned ID의 허용 byte 하나가 달라지면 digest도 달라지는지 검증한다.
#[test]
fn digest_canonicalizes_only_json_values_and_preserves_validated_string_bytes() {
    let left = profile_with_parameters(
        json!({"b": 2, "a": 1}),
        json!({"nested": {"z": false, "a": true}}),
    );
    let right = profile_with_parameters(
        json!({"a": 1, "b": 2}),
        json!({"nested": {"a": true, "z": false}}),
    );
    assert_eq!(left.digest(), right.digest());

    let distinct_identifier = BindingProfileV1::new(
        left.normalized_endpoint().clone(),
        left.api_dialect(),
        left.resolved_connector_id().clone(),
        "cl100k-base/v1",
        left.context_limit(),
        left.output_limit(),
        left.reasoning_parameters().clone(),
        left.optional_request_parameters().clone(),
        left.tool_capability_policy(),
        left.verification_profile(),
    )
    .unwrap();
    assert_ne!(left.digest(), distinct_identifier.digest());
}

// endpoint의 url 2.5.8 normalization 결과가 같으면 입력 표기가 달라도 digest가 같고,
// v1의 각 독립 의미 field가 달라지면 새 binding identity digest가 생기는지 검증한다.
#[test]
fn every_semantic_field_controls_the_digest_after_endpoint_normalization() {
    let base = ProfileFixture::default();
    let base_profile = base.build();
    let canonical_endpoint = ProfileFixture {
        endpoint: "https://example.com/v1".to_owned(),
        ..base.clone()
    }
    .build();
    assert_eq!(base_profile.digest(), canonical_endpoint.digest());

    let mut variants = Vec::new();
    let mut changed = base.clone();
    changed.endpoint = "https://example.com/v2".to_owned();
    variants.push(("normalized_endpoint", changed));
    let mut changed = base.clone();
    changed.dialect = ApiDialect::OpenAiChatCompletions;
    variants.push(("api_dialect and resolved_connector_id", changed));
    let mut changed = base.clone();
    changed.tokenizer_profile = "o200k_base/v1".to_owned();
    variants.push(("tokenizer_profile", changed));
    let mut changed = base.clone();
    changed.context_limit = 128_001;
    variants.push(("context_limit", changed));
    let mut changed = base.clone();
    changed.output_limit = 16_385;
    variants.push(("output_limit", changed));
    let mut changed = base.clone();
    changed.reasoning_parameters = json!({"effort": "high"});
    variants.push(("reasoning_parameters", changed));
    let mut changed = base.clone();
    changed.optional_request_parameters = json!({"temperature": 0.5});
    variants.push(("optional_request_parameters", changed));
    let mut changed = base.clone();
    changed.tool_capability_policy = "tools.required/v1".to_owned();
    variants.push(("tool_capability_policy", changed));
    let mut changed = base;
    changed.verification_profile = "semantic-terminal/v3".to_owned();
    variants.push(("verification_profile", changed));

    for (field, variant) in variants {
        assert_ne!(
            base_profile.digest(),
            variant.build().digest(),
            "changing {field} must change the digest"
        );
    }
}

// RFC 8785는 number를 ECMAScript 형식으로 쓰고 key를 UTF-16 code unit 순서로 정렬하며
// string을 Unicode-normalize하지 않으므로 그 byte와 조합형/완성형 digest 차이를 보존한다.
#[test]
fn rfc8785_preserves_unicode_form_and_uses_jcs_number_and_key_order() {
    let decomposed = profile_with_parameters(
        json!({
            "\u{fb33}": "Hebrew",
            "😀": "Emoji",
            "€": "Euro",
            "unnormalized": "A\u{030a}",
            "numbers": [333_333_333.333_333_3_f64, 1e30, 4.50, 2e-3, 1e-27],
        }),
        Value::Null,
    );
    assert_eq!(
        framed_value(&decomposed, b"reasoning_parameters"),
        "{\"numbers\":[333333333.3333333,1e+30,4.5,0.002,1e-27],\"unnormalized\":\"A\u{030a}\",\"€\":\"Euro\",\"😀\":\"Emoji\",\"\u{fb33}\":\"Hebrew\"}"
            .as_bytes()
    );

    let composed = profile_with_parameters(
        json!({
            "\u{fb33}": "Hebrew",
            "😀": "Emoji",
            "€": "Euro",
            "unnormalized": "Å",
            "numbers": [333_333_333.333_333_3_f64, 1e30, 4.50, 2e-3, 1e-27],
        }),
        Value::Null,
    );
    assert_ne!(decomposed.digest(), composed.digest());
}

// JSON array는 RFC 8785에서도 순서를 보존하므로 element 순서만 다른 두 structured
// parameter가 같은 profile digest로 합쳐지지 않는지 검증한다.
#[test]
fn json_array_order_changes_the_digest() {
    let left = profile_with_parameters(json!([1, 2, 3]), Value::Null);
    let right = profile_with_parameters(json!([3, 2, 1]), Value::Null);

    assert_ne!(left.digest(), right.digest());
}

// versioned profile ID의 128-byte 상한은 registry 상태와 무관하게 허용하고, ASCII·name·
// version 문법이나 상한을 벗어난 값은 profile identity를 만들기 전에 모두 거절한다.
#[test]
fn versioned_profile_ids_use_the_closed_ascii_grammar_and_exact_byte_limit() {
    let longest = format!("{}/v1", "a".repeat(125));
    for position in 0..3 {
        let mut fixture = ProfileFixture {
            context_limit: 2,
            output_limit: 1,
            ..ProfileFixture::default()
        };
        match position {
            0 => fixture.tokenizer_profile.clone_from(&longest),
            1 => fixture.tool_capability_policy.clone_from(&longest),
            2 => fixture.verification_profile.clone_from(&longest),
            _ => unreachable!(),
        }
        let profile = fixture.build();
        let accepted = match position {
            0 => profile.tokenizer_profile(),
            1 => profile.tool_capability_policy(),
            2 => profile.verification_profile(),
            _ => unreachable!(),
        };
        assert_eq!(accepted.len(), 128);
    }

    let overlong = format!("{}/v1", "a".repeat(126));
    for position in 0..3 {
        let mut values = ["tokenizer/v1", "tools.none/v1", "semantic-terminal/v1"];
        values[position] = &overlong;
        assert!(
            BindingProfileV1::new(
                NormalizedEndpoint::parse("https://example.com/v1").unwrap(),
                ApiDialect::OpenAiResponses,
                ConnectorId::for_dialect(ApiDialect::OpenAiResponses),
                values[0],
                2,
                1,
                Value::Null,
                Value::Null,
                values[1],
                values[2],
            )
            .is_err()
        );
    }

    for invalid in [
        "a/v0".to_owned(),
        "a/v01".to_owned(),
        "A/v1".to_owned(),
        "-a/v1".to_owned(),
        "a/v".to_owned(),
        "a/v1/extra".to_owned(),
        "é/v1".to_owned(),
    ] {
        assert!(
            BindingProfileV1::new(
                NormalizedEndpoint::parse("https://example.com/v1").unwrap(),
                ApiDialect::OpenAiResponses,
                ConnectorId::for_dialect(ApiDialect::OpenAiResponses),
                invalid,
                2,
                1,
                Value::Null,
                Value::Null,
                "tools.none/v1",
                "semantic-terminal/v1",
            )
            .is_err()
        );
    }
}

// public digest parser는 canonical lowercase sha256 syntax 하나만 받아 durable caller가
// 대문자, 길이 오류, non-hex 값을 동일한 digest identity로 오인하지 않게 한다.
#[test]
fn binding_profile_digest_parser_accepts_only_exact_lowercase_sha256_syntax() {
    let exact = format!("sha256:{}", "0123456789abcdef".repeat(4));
    let parsed = exact.parse::<BindingProfileDigest>().unwrap();
    assert_eq!(parsed.as_str(), exact);

    for invalid in [
        format!("sha256:{}", "0123456789abcdef".repeat(3)),
        format!("sha256:{}0", "0123456789abcdef".repeat(4)),
        format!("sha256:{}", "0123456789abcdeF".repeat(4)),
        format!("sha256:{}g", "0".repeat(63)),
        format!("sha512:{}", "0".repeat(64)),
    ] {
        assert!(invalid.parse::<BindingProfileDigest>().is_err());
    }
}

// v1 identity는 limit의 실행 가능성을 판단하지 않고 모든 u64를 선행 0 없는 십진수로
// 보존하므로 0과 역전된 값도 byte identity로 만들며, 런타임 admission은 별도로 둔다.
#[test]
fn limit_identity_preserves_zero_and_runtime_invalid_values_as_decimal_bytes() {
    let profile = BindingProfileV1::new(
        NormalizedEndpoint::parse("https://example.com/v1").unwrap(),
        ApiDialect::OpenAiResponses,
        ConnectorId::for_dialect(ApiDialect::OpenAiResponses),
        "tokenizer/v1",
        0,
        u64::MAX,
        Value::Null,
        Value::Null,
        "tools.none/v1",
        "semantic-terminal/v1",
    )
    .unwrap();
    assert_eq!(framed_value(&profile, b"context_limit"), b"0");
    assert_eq!(
        framed_value(&profile, b"output_limit"),
        u64::MAX.to_string().as_bytes()
    );
}

// connector는 dialect에서 정확히 하나로 파생되므로 서로 다른 두 값을 함께 넣으면
// 같은 profile identity가 지원하지 않는 실행 조합을 주장하기 전에 거절한다.
#[test]
fn rejects_connector_mismatch() {
    assert!(
        BindingProfileV1::new(
            NormalizedEndpoint::parse("https://example.com/v1").unwrap(),
            ApiDialect::OpenAiResponses,
            ConnectorId::for_dialect(ApiDialect::OpenAiChatCompletions),
            "tokenizer/v1",
            2,
            1,
            Value::Null,
            Value::Null,
            "tools.none/v1",
            "semantic-terminal/v1",
        )
        .is_err()
    );
}
