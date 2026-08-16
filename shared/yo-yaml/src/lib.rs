//! Shared, bounded YAML serialization for Yo workspace components.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
pub use serde_saphyr::{Error, SerializeError};

const MAX_YAML_EVENTS: usize = 100_000;
const MAX_YAML_NODES: usize = 50_000;
const MAX_YAML_ALIASES: usize = 1_024;
const MAX_YAML_ANCHORS: usize = 1_024;
const MAX_YAML_DEPTH: usize = 64;
const DEFAULT_MAX_YAML_SCALAR_BYTES: usize = 2 * 1024 * 1024;
const MAX_ALIAS_REPLAY_EVENTS: usize = 100_000;
const MAX_ALIAS_REPLAY_DEPTH: usize = 32;
const MAX_EXPANSIONS_PER_ANCHOR: usize = 1_024;

#[derive(Clone, Copy)]
struct ParserLimits {
    flow_nesting: usize,
    events: usize,
    nodes: usize,
    aliases: usize,
    anchors: usize,
    depth: usize,
    replay_events: usize,
    replay_depth: usize,
    expansions_per_anchor: usize,
}

const PARSER_LIMITS: ParserLimits = ParserLimits {
    flow_nesting: MAX_YAML_DEPTH,
    events: MAX_YAML_EVENTS,
    nodes: MAX_YAML_NODES,
    aliases: MAX_YAML_ALIASES,
    anchors: MAX_YAML_ANCHORS,
    depth: MAX_YAML_DEPTH,
    replay_events: MAX_ALIAS_REPLAY_EVENTS,
    replay_depth: MAX_ALIAS_REPLAY_DEPTH,
    expansions_per_anchor: MAX_EXPANSIONS_PER_ANCHOR,
};

/// Per-input limits that consumers may narrow without changing shared YAML semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseLimits {
    max_total_scalar_bytes: usize,
}

impl ParseLimits {
    /// Uses the workspace defaults except for the total scalar byte budget.
    #[must_use]
    pub const fn with_max_total_scalar_bytes(max_total_scalar_bytes: usize) -> Self {
        Self {
            max_total_scalar_bytes,
        }
    }
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self::with_max_total_scalar_bytes(DEFAULT_MAX_YAML_SCALAR_BYTES)
    }
}

fn deserialize_options(limits: ParseLimits) -> serde_saphyr::Options {
    deserialize_options_with_parser_limits(limits, PARSER_LIMITS)
}

fn deserialize_options_with_parser_limits(
    limits: ParseLimits,
    parser_limits: ParserLimits,
) -> serde_saphyr::Options {
    serde_saphyr::options! {
        budget: serde_saphyr::budget! {
            flow_nesting_limit: parser_limits.flow_nesting,
            max_events: parser_limits.events,
            max_aliases: parser_limits.aliases,
            max_anchors: parser_limits.anchors,
            max_depth: parser_limits.depth,
            max_documents: 1,
            max_nodes: parser_limits.nodes,
            max_total_scalar_bytes: limits.max_total_scalar_bytes,
        },
        emit_comments: false,
        merge_keys: serde_saphyr::MergeKeyPolicy::Error,
        alias_limits: serde_saphyr::alias_limits! {
            max_total_replayed_events: parser_limits.replay_events,
            max_replay_stack_depth: parser_limits.replay_depth,
            max_alias_expansions_per_anchor: parser_limits.expansions_per_anchor,
        },
    }
}

/// Deserializes one YAML document with the shared workspace limits and semantics.
pub fn from_str<'de, T>(contents: &'de str) -> Result<T, Error>
where
    T: Deserialize<'de>,
{
    from_str_with_limits(contents, ParseLimits::default())
}

/// Deserializes one YAML document with a consumer-specific scalar byte budget.
pub fn from_str_with_limits<'de, T>(contents: &'de str, limits: ParseLimits) -> Result<T, Error>
where
    T: Deserialize<'de>,
{
    serde_saphyr::from_str_with_options(contents, deserialize_options(limits))
}

/// Deserializes one YAML document from bytes with the shared workspace limits and semantics.
pub fn from_slice<'de, T>(contents: &'de [u8]) -> Result<T, Error>
where
    T: Deserialize<'de>,
{
    serde_saphyr::from_slice_with_options(contents, deserialize_options(ParseLimits::default()))
}

/// Reports whether the root mapping contains any exact key from `field_names`.
///
/// Values are structurally consumed under the same parser budgets but are not materialized. This
/// keeps format-retirement diagnostics scoped to root fields instead of matching an identically
/// named nested field from a typed deserialization error.
pub fn has_any_top_level_mapping_key(contents: &[u8], field_names: &[&str]) -> Result<bool, Error> {
    let fields: BTreeMap<String, serde::de::IgnoredAny> = serde_saphyr::from_slice_with_options(
        contents,
        deserialize_options(ParseLimits::default()),
    )?;
    Ok(field_names.iter().any(|field| fields.contains_key(*field)))
}

/// Serializes a value with the workspace YAML backend.
pub fn to_string<T>(value: &T) -> Result<String, SerializeError>
where
    T: Serialize,
{
    serde_saphyr::to_string(value)
}

#[cfg(test)]
mod tests {
    fn parse_with_parser_limits(
        contents: &str,
        parser_limits: super::ParserLimits,
    ) -> Result<serde_json::Value, super::Error> {
        serde_saphyr::from_str_with_options(
            contents,
            super::deserialize_options_with_parser_limits(
                super::ParseLimits::default(),
                parser_limits,
            ),
        )
    }

    // 작은 alias는 허용하되 merge key는 공통 정책에서 거부합니다.
    #[test]
    fn rejects_merge_keys_but_allows_small_aliases() {
        let alias: serde_json::Value = super::from_str("base: &base [one]\ncopy: *base\n").unwrap();
        assert_eq!(alias["copy"], serde_json::json!(["one"]));

        let error = super::from_str::<serde_json::Value>(
            "base: &base {one: 1}\nmerged: {<<: *base, two: 2}\n",
        )
        .unwrap_err();
        assert!(error.to_string().contains("merge"), "{error}");
    }

    // 편의 표기를 포함한 serde-saphyr 기본 boolean inference를 그대로 제공합니다.
    #[test]
    fn preserves_backend_boolean_inference() {
        let value: serde_json::Value =
            super::from_str("yes_value: yes\nno_value: no\non_value: ON\noff_value: Off\n")
                .unwrap();
        assert_eq!(value["yes_value"], serde_json::json!(true));
        assert_eq!(value["no_value"], serde_json::json!(false));
        assert_eq!(value["on_value"], serde_json::json!(true));
        assert_eq!(value["off_value"], serde_json::json!(false));
    }

    // 숫자 scalar는 별도 전처리 없이 serde-saphyr의 inference를 그대로 사용합니다.
    #[test]
    fn preserves_backend_number_inference() {
        let value: serde_json::Value = super::from_str("value: 1_000\n").unwrap();
        assert_eq!(value["value"], serde_json::json!(1000));
    }

    // 64-bit integer 범위를 벗어난 decimal은 유한 float로, float 문법이 아닌 radix
    // overflow는 String으로 남는 backend 순서를 정확한 variant로 고정합니다.
    #[test]
    fn pins_integer_range_and_float_fallback_variants() {
        let value: serde_json::Value = super::from_str(concat!(
            "u64_max: 18446744073709551615\n",
            "u64_overflow: 18446744073709551616\n",
            "large_decimal: 340282366920938463463374607431768211456\n",
            "i64_min: -9223372036854775808\n",
            "i64_underflow: -9223372036854775809\n",
            "hex_max: 0xffffffffffffffff\n",
            "hex_overflow: 0x10000000000000000\n",
            "leading_dot_overflow: .5e400\n",
            "negative_leading_dot_overflow: -.5e400\n",
        ))
        .unwrap();

        assert_eq!(value["u64_max"], serde_json::json!(u64::MAX));
        assert_eq!(
            value["u64_overflow"],
            serde_json::json!(18_446_744_073_709_551_616.0_f64)
        );
        assert_eq!(
            value["large_decimal"],
            serde_json::json!(340_282_366_920_938_463_463_374_607_431_768_211_456.0_f64)
        );
        assert_eq!(value["i64_min"], serde_json::json!(i64::MIN));
        assert_eq!(
            value["i64_underflow"],
            serde_json::json!(-9_223_372_036_854_775_809.0_f64)
        );
        assert_eq!(value["hex_max"], serde_json::json!(u64::MAX));
        assert_eq!(value["hex_overflow"], "0x10000000000000000");
        assert_eq!(value["leading_dot_overflow"], ".5e400");
        assert_eq!(value["negative_leading_dot_overflow"], "-.5e400");
    }

    // 퇴역 형상 판정은 같은 이름의 중첩 key를 루트 key로 오인하지 않습니다.
    #[test]
    fn detects_only_top_level_mapping_keys() {
        assert!(
            super::has_any_top_level_mapping_key(
                b"version: 1\nnested: {profile_digests: []}\n",
                &["version", "profile_digests"],
            )
            .unwrap()
        );
        assert!(
            !super::has_any_top_level_mapping_key(
                b"nested: {version: 1, profile_digests: []}\n",
                &["version", "profile_digests"],
            )
            .unwrap()
        );
    }

    // 소비자는 공통 의미를 바꾸지 않고 입력별 scalar byte budget만 좁힐 수 있습니다.
    #[test]
    fn permits_a_narrower_scalar_byte_budget() {
        let error = super::from_str_with_limits::<String>(
            "four",
            super::ParseLimits::with_max_total_scalar_bytes(3),
        )
        .unwrap_err();
        assert!(error.to_string().contains("scalar"), "{error}");
    }

    // 같은 mapping의 key가 두 번 나오면 뒤의 값을 택하지 않고 문서 전체를 거절합니다.
    #[test]
    fn rejects_duplicate_mapping_keys() {
        let error = super::from_str::<serde_json::Value>("value: one\nvalue: two\n").unwrap_err();
        assert!(error.to_string().contains("duplicate"), "{error}");
    }

    // 단일 문서 entrypoint는 두 번째 document marker 뒤의 값을 무시하지 않습니다.
    #[test]
    fn rejects_additional_documents() {
        let error = super::from_str::<serde_json::Value>("---\none\n---\ntwo\n").unwrap_err();
        assert!(
            error.to_string().to_lowercase().contains("document"),
            "{error}"
        );
    }

    // 정의되지 않은 alias와 자기 자신을 재생하는 cycle은 typed 값으로 만들지 않습니다.
    #[test]
    fn rejects_unknown_and_cyclic_aliases() {
        assert!(super::from_str::<serde_json::Value>("value: *missing\n").is_err());
        assert!(super::from_str::<serde_json::Value>("value: &self [*self]\n").is_err());
    }

    // event budget은 node budget과 독립적으로 parser stream을 중단합니다.
    #[test]
    fn enforces_event_budget() {
        let limits = super::ParserLimits {
            events: 3,
            ..super::PARSER_LIMITS
        };
        let error = parse_with_parser_limits("[one, two, three]\n", limits).unwrap_err();
        assert!(error.to_string().contains("event"), "{error}");
    }

    // node budget은 event budget에 여유가 있어도 materialized value 크기를 제한합니다.
    #[test]
    fn enforces_node_budget() {
        let limits = super::ParserLimits {
            events: 100,
            nodes: 2,
            ..super::PARSER_LIMITS
        };
        let error = parse_with_parser_limits("[one, two, three]\n", limits).unwrap_err();
        assert!(error.to_string().contains("node"), "{error}");
    }

    // block container도 공통 maximum depth를 넘으면 재귀 역직렬화를 시작하지 않습니다.
    #[test]
    fn enforces_depth_budget() {
        let limits = super::ParserLimits {
            depth: 2,
            ..super::PARSER_LIMITS
        };
        let error = parse_with_parser_limits("-\n  -\n    - value\n", limits).unwrap_err();
        assert!(error.to_string().contains("depth"), "{error}");
    }

    // anchor 정의 수와 alias 사용 수는 각각 독립된 structural budget입니다.
    #[test]
    fn enforces_anchor_and_alias_budgets() {
        let anchor_limits = super::ParserLimits {
            anchors: 1,
            ..super::PARSER_LIMITS
        };
        let anchor_error =
            parse_with_parser_limits("one: &one 1\ntwo: &two 2\n", anchor_limits).unwrap_err();
        assert!(
            anchor_error.to_string().contains("anchor"),
            "{anchor_error}"
        );

        let alias_limits = super::ParserLimits {
            aliases: 1,
            ..super::PARSER_LIMITS
        };
        let alias_error =
            parse_with_parser_limits("base: &base 1\none: *base\ntwo: *base\n", alias_limits)
                .unwrap_err();
        assert!(alias_error.to_string().contains("alias"), "{alias_error}");
    }

    // replay 총량, 재생 stack 깊이, anchor별 expansion 횟수는 일반 alias 수와 별도로
    // 제한되어 작은 alias 지원이 증폭 입력 허용으로 바뀌지 않습니다.
    #[test]
    fn enforces_alias_replay_budgets() {
        let replay_limits = super::ParserLimits {
            replay_events: 2,
            ..super::PARSER_LIMITS
        };
        let replay_error =
            parse_with_parser_limits("base: &base [one, two]\ncopy: *base\n", replay_limits)
                .unwrap_err();
        assert!(
            replay_error.to_string().contains("replay"),
            "{replay_error}"
        );

        let depth_limits = super::ParserLimits {
            replay_depth: 0,
            ..super::PARSER_LIMITS
        };
        let depth_error =
            parse_with_parser_limits("base: &base [value]\ncopy: *base\n", depth_limits)
                .unwrap_err();
        assert!(depth_error.to_string().contains("depth"), "{depth_error}");

        let expansion_limits = super::ParserLimits {
            expansions_per_anchor: 1,
            ..super::PARSER_LIMITS
        };
        let expansion_error = parse_with_parser_limits(
            "base: &base one\nfirst: *base\nsecond: *base\n",
            expansion_limits,
        )
        .unwrap_err();
        assert!(
            expansion_error.to_string().contains("expansion"),
            "{expansion_error}"
        );
    }

    // null은 plain spelling에서만 추론되고 quoted null과 quoted boolean/number는 String입니다.
    #[test]
    fn resolves_plain_nulls_before_other_scalars_and_preserves_quotes() {
        let value: serde_json::Value = super::from_str(
            "empty:\ntilde: ~\nlower: null\nupper: NULL\nquoted_null: 'null'\nquoted_bool: \"yes\"\nquoted_number: '1_000'\n",
        )
        .unwrap();
        assert!(value["empty"].is_null());
        assert!(value["tilde"].is_null());
        assert!(value["lower"].is_null());
        assert!(value["upper"].is_null());
        assert_eq!(value["quoted_null"], "null");
        assert_eq!(value["quoted_bool"], "yes");
        assert_eq!(value["quoted_number"], "1_000");
    }

    // digit separator가 숫자 사이에 있을 때만 integer로 추론하고 잘못 놓인 underscore는
    // 소비자가 그대로 확인할 수 있는 String으로 남깁니다.
    #[test]
    fn accepts_only_between_digit_integer_separators() {
        let value: serde_json::Value =
            super::from_str("valid: 1_000\nleading: _1000\ntrailing: 1000_\ndoubled: 1__000\n")
                .unwrap();
        assert_eq!(value["valid"], serde_json::json!(1000));
        assert_eq!(value["leading"], "_1000");
        assert_eq!(value["trailing"], "1000_");
        assert_eq!(value["doubled"], "1__000");
    }

    // typeless non-finite scalar는 String으로 낮추지 않고 parser 경계에서 거절합니다.
    #[test]
    fn rejects_non_finite_inferred_numbers() {
        for scalar in [".nan", ".inf", "-.inf", "1e999"] {
            assert!(
                super::from_str::<serde_json::Value>(scalar).is_err(),
                "{scalar}"
            );
        }
    }

    // f64 직렬화는 정수와 구분되는 float spelling을 남겨 variant-exact round trip을
    // 유지합니다.
    #[test]
    fn serializes_floats_with_float_spelling() {
        let encoded = super::to_string(&serde_json::json!({"value": 1.0})).unwrap();
        assert!(encoded.contains("value: 1.0"), "{encoded}");
    }
}
