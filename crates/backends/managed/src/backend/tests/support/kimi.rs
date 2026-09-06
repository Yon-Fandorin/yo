use yo_core::{
    AccountId, ApiDialect, EffectiveModelBinding, ModelId, ModelProfileLayer,
    ModelProfileParameters, ModelReplayItem, ModelReplayRole, NormalizedEndpoint, ProviderId,
    ProviderPrivateReplayEnvelope, VersionedProfileId,
};

pub(in crate::backend) fn kimi_admission(
    complete: &yo_core::CompleteModelBinding,
) -> Result<yo_core::AdmittedCompleteBinding, String> {
    yo_connector_kimi::admit_complete_binding(complete).map_err(|error| error.to_string())
}

pub(in crate::backend) fn private_envelope(
    private: &str,
    visible: Option<&str>,
) -> ProviderPrivateReplayEnvelope {
    let reasoning = serde_json::to_string(private).unwrap();
    let content = visible.map_or_else(
        || "null".to_owned(),
        |visible| serde_json::to_string(visible).unwrap(),
    );
    ProviderPrivateReplayEnvelope::new(
        "kimi.assistant-message/v1alpha1",
        format!(r#"{{"role":"assistant","reasoning_content":{reasoning},"content":{content}}}"#)
            .into_bytes(),
    )
    .unwrap()
}

pub(in crate::backend) fn kimi_binding() -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("kimi").unwrap(),
        AccountId::new("team").unwrap(),
        ModelId::new("kimi-k3").unwrap(),
        ApiDialect::KimiChatCompletions,
        NormalizedEndpoint::parse("https://api.moonshot.ai/v1").unwrap(),
    )
}

pub(in crate::backend) fn kimi_profile() -> yo_core::EffectiveModelProfile {
    let layer = ModelProfileLayer::new(
        Some(ApiDialect::KimiChatCompletions),
        Some(VersionedProfileId::new("utf8-bytes/v1").unwrap()),
        Some(1_048_576),
        Some(131_072),
        Some(serde_json::from_str::<ModelProfileParameters>(r#"{"effort":"max"}"#).unwrap()),
        Some(serde_json::from_str::<ModelProfileParameters>("{}").unwrap()),
        Some(VersionedProfileId::new("local-tools/v1").unwrap()),
    )
    .with_replay_profile(Some(
        VersionedProfileId::new("kimi-private-local-plaintext/v1").unwrap(),
    ));
    yo_core::EffectiveModelProfile::resolve(None, &layer).unwrap()
}

pub(in crate::backend) fn visible_message(content: impl Into<String>) -> ModelReplayItem {
    ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content: content.into(),
        refusal: None,
    }
}
