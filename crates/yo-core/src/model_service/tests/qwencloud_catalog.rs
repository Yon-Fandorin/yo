use super::super::{
    AccountId, ModelId, ProviderId, QwenCloudCatalogAvailability, QwenCloudCatalogDisabledReason,
    QwenCloudCatalogSeed, VersionedProfileId,
};

fn seed(profile: &str) -> QwenCloudCatalogSeed {
    QwenCloudCatalogSeed::resolve(
        VersionedProfileId::new(profile).unwrap(),
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("team").unwrap(),
        None,
        Some("Team".to_owned()),
    )
    .unwrap()
}

// 두 Coding Plan 지역은 endpoint만 다르고 공식 exact allowlist와 완전한 실행 profile은
// 동일하며, picker inventory는 표시명 case-fold 후 exact ModelId 순으로 정렬되는지 검증합니다.
#[test]
fn qwencloud_catalog_coding_plans_have_exact_rows_and_region_endpoints() {
    let china = seed("qwencloud-coding-plan-cn/v1");
    let international = seed("qwencloud-coding-plan-intl/v1");
    let expected = [
        "glm-4.7",
        "glm-5",
        "kimi-k2.5",
        "MiniMax-M2.5",
        "qwen3-coder-next",
        "qwen3-coder-plus",
        "qwen3-max-2026-01-23",
        "qwen3.5-plus",
        "qwen3.6-plus",
        "qwen3.7-plus",
    ];
    assert_eq!(china.models().len(), expected.len());
    assert_eq!(international.models().len(), expected.len());
    for (china, (international, expected)) in china
        .models()
        .iter()
        .zip(international.models().iter().zip(expected))
    {
        assert_eq!(china.model_id().as_str(), expected);
        assert_eq!(international.model_id().as_str(), expected);
        assert!(china.is_enabled());
        assert!(international.is_enabled());
        assert_eq!(
            china.entry().unwrap().binding().endpoint().as_str(),
            "https://coding.dashscope.aliyuncs.com/v1"
        );
        assert_eq!(
            international.entry().unwrap().binding().endpoint().as_str(),
            "https://coding-intl.dashscope.aliyuncs.com/v1"
        );
    }
}

// catalog의 모델별 context override와 공통 보수적 output cap, Chat Completions dialect,
// 도구·검증 profile이 완전한 durable binding에 함께 고정되는지 판별합니다.
#[test]
fn qwencloud_catalog_resolves_complete_model_specific_profiles() {
    let seed = seed("qwencloud-coding-plan-intl/v1");
    let coder = seed
        .model(&ModelId::new("qwen3-coder-next").unwrap())
        .unwrap();
    let complete = coder.entry().unwrap().complete_binding().unwrap();
    assert_eq!(
        complete.binding().connector_id().as_str(),
        "openai-chat-completions"
    );
    assert_eq!(complete.profile().context().input_token_limit(), 262_144);
    assert_eq!(complete.profile().context().max_output_tokens(), 8_192);
    assert_eq!(
        complete.profile().context().tokenizer_profile(),
        "utf8-bytes/v1"
    );
    assert!(complete.profile().reasoning_parameters().is_empty_mapping());
    assert!(
        complete
            .profile()
            .optional_request_parameters()
            .is_empty_mapping()
    );
    assert_eq!(
        complete.profile().tool_capability_policy().as_str(),
        "local-tools/v1"
    );
    assert_eq!(
        complete.profile().verification_profile().as_str(),
        "semantic-terminal/v1"
    );
    assert_eq!(coder.reasoning(), Some(true));
}

// Token Plan의 공식 image-generation 행도 목록에서 사라지지 않지만 현재 text-agent
// protocol로 임의 실행되지 않고 안정적인 disabled reason과 함께 남는지 검증합니다.
#[test]
fn qwencloud_token_plan_keeps_incompatible_rows_visible_but_non_routable() {
    let seed = seed("qwencloud-token-plan-team-intl/v1");
    assert_eq!(seed.models().len(), 18);
    let image = seed
        .model(&ModelId::new("qwen-image-2.0").unwrap())
        .unwrap();
    assert_eq!(image.entry(), None);
    assert_eq!(
        image.availability(),
        QwenCloudCatalogAvailability::Disabled(
            QwenCloudCatalogDisabledReason::TextOutputUnsupported
        )
    );
    assert_eq!(
        QwenCloudCatalogDisabledReason::TextOutputUnsupported.as_str(),
        "text output unsupported"
    );
    assert_eq!(image.input_limit(), Some(1_000_000));
    assert_eq!(image.output_limit(), None);
    assert_eq!(image.tool_policy(), None);
    assert_eq!(image.reasoning(), None);
    let text = seed.model(&ModelId::new("deepseek-v3.2").unwrap()).unwrap();
    assert!(text.entry().is_some());
    assert_eq!(text.input_limit(), Some(131_072));
}

// 닫힌 registry는 알 수 없는 profile과 다른 Provider의 재사용을 URL 추론이나 fallback
// 없이 거절하고, 생략한 Provider display에만 QwenCloud fallback을 적용합니다.
#[test]
fn qwencloud_catalog_rejects_unknown_or_mismatched_profiles() {
    let unknown = QwenCloudCatalogSeed::resolve(
        VersionedProfileId::new("qwencloud-future/v1").unwrap(),
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("team").unwrap(),
        None,
        None,
    )
    .unwrap_err();
    assert!(
        unknown
            .to_string()
            .contains("unknown built-in catalog profile")
    );

    let mismatch = QwenCloudCatalogSeed::resolve(
        VersionedProfileId::new("qwencloud-coding-plan-intl/v1").unwrap(),
        ProviderId::new("vendor").unwrap(),
        AccountId::new("team").unwrap(),
        None,
        None,
    )
    .unwrap_err();
    assert!(
        mismatch
            .to_string()
            .contains("requires ProviderId qwencloud")
    );

    let seed = seed("qwencloud-coding-plan-intl/v1");
    assert_eq!(
        seed.models()[0].entry().unwrap().provider_display_name(),
        Some("QwenCloud")
    );
}
