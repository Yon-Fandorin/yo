use serde_json::json;

use super::*;

// Kimi의 cached_tokens 부재는 unsupported나 0으로 바꾸지 않고 absent로 남기며,
// 음수·null·prompt 초과 값은 Provider 보고값을 추측해 고치지 않고 거절합니다.
#[test]
fn kimi_usage_distinguishes_absent_cache_reads_and_rejects_invalid_reports() {
    let usage = json!({"prompt_tokens":4,"completion_tokens":3,"total_tokens":7});
    let decoded = decode_usage(&usage).unwrap();
    assert!(matches!(
        decoded.cache_read_input_tokens,
        yo_core::CacheReadInputTokens::Absent { ref source_profile }
            if source_profile.as_str() == "kimi.usage.cached-tokens/v1"
    ));

    for cached_tokens in [json!(-1), Value::Null, json!(5)] {
        let mut invalid = usage.clone();
        invalid["cached_tokens"] = cached_tokens;
        assert!(decode_usage(&invalid).is_err());
    }
}
