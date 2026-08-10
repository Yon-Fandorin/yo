use crate::{
    AccountId, ApiDialect, EffectiveModelBinding, ModelId, NormalizedEndpoint, ProviderId,
};

pub(super) fn qwen_binding() -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("qwencloud-token-plan").unwrap(),
        ModelId::new("qwen3.8max").unwrap(),
        ApiDialect::OpenAiResponses,
        NormalizedEndpoint::parse(
            "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1",
        )
        .unwrap(),
    )
}

pub(super) fn event(value: serde_json::Value) -> String {
    format!("data: {}\n\n", serde_json::to_string(&value).unwrap())
}
