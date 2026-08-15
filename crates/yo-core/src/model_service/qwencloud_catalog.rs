use std::collections::HashSet;

use super::{
    AccountId, ApiDialect, EffectiveModelBinding, EffectiveModelProfile, ModelCatalogEntry,
    ModelId, ModelProfileLayer, ModelProfileParameters, ModelServiceError, NormalizedEndpoint,
    ProviderId, VersionedProfileId,
};

const QWENCLOUD_PROVIDER: &str = "qwencloud";
const PROVIDER_DISPLAY_FALLBACK: &str = "QwenCloud";
const MAX_CATALOG_ROWS: usize = 4_096;
const DEFAULT_INPUT_LIMIT: u64 = 1_000_000;
const DEFAULT_OUTPUT_LIMIT: u64 = 8_192;

const CODING_PLAN_ROWS: &[CatalogRow] = &[
    text_agent("qwen3.7-plus", 1_000_000),
    text_agent("qwen3.6-plus", 1_000_000),
    text_agent("kimi-k2.5", 262_144),
    text_agent("glm-5", 202_752),
    text_agent("MiniMax-M2.5", 196_608),
    text_agent("qwen3.5-plus", 1_000_000),
    text_agent("qwen3-max-2026-01-23", 262_144),
    text_agent("qwen3-coder-next", 262_144),
    text_agent("qwen3-coder-plus", 1_000_000),
    text_agent("glm-4.7", 202_752),
];

const TOKEN_PLAN_ROWS: &[CatalogRow] = &[
    text_agent("qwen3.7-max", 1_000_000),
    text_agent("qwen3.7-plus", 1_000_000),
    text_agent("qwen3.6-plus", 1_000_000),
    text_agent("qwen3.6-flash", 1_000_000),
    image_generation("qwen-image-2.0"),
    image_generation("qwen-image-2.0-pro"),
    image_generation("wan2.7-image"),
    image_generation("wan2.7-image-pro"),
    text_agent("deepseek-v4-pro", 1_000_000),
    text_agent("deepseek-v4-flash", 1_000_000),
    text_agent("deepseek-v3.2", 131_072),
    text_agent("kimi-k2.7-code", 262_144),
    text_agent("kimi-k2.6", 262_144),
    text_agent("kimi-k2.5", 262_144),
    text_agent("glm-5.2", 202_752),
    text_agent("glm-5.1", 202_752),
    text_agent("glm-5", 202_752),
    text_agent("MiniMax-M2.5", 196_608),
];

#[derive(Clone, Copy)]
struct CatalogDefinition {
    profile: &'static str,
    endpoint: &'static str,
    rows: &'static [CatalogRow],
}

const CATALOGS: &[CatalogDefinition] = &[
    CatalogDefinition {
        profile: "qwencloud-coding-plan-cn/v1",
        endpoint: "https://coding.dashscope.aliyuncs.com/v1/",
        rows: CODING_PLAN_ROWS,
    },
    CatalogDefinition {
        profile: "qwencloud-coding-plan-intl/v1",
        endpoint: "https://coding-intl.dashscope.aliyuncs.com/v1/",
        rows: CODING_PLAN_ROWS,
    },
    CatalogDefinition {
        profile: "qwencloud-token-plan-team-intl/v1",
        endpoint: "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/",
        rows: TOKEN_PLAN_ROWS,
    },
];

#[derive(Clone, Copy)]
struct CatalogRow {
    model: &'static str,
    text_input: bool,
    text_output: bool,
    tools: bool,
    reasoning: Option<bool>,
    input_limit: u64,
    output_limit: u64,
}

const fn text_agent(model: &'static str, input_limit: u64) -> CatalogRow {
    CatalogRow {
        model,
        text_input: true,
        text_output: true,
        tools: true,
        reasoning: Some(true),
        input_limit,
        output_limit: DEFAULT_OUTPUT_LIMIT,
    }
}

const fn image_generation(model: &'static str) -> CatalogRow {
    CatalogRow {
        model,
        text_input: true,
        text_output: false,
        tools: false,
        reasoning: None,
        input_limit: DEFAULT_INPUT_LIMIT,
        output_limit: DEFAULT_OUTPUT_LIMIT,
    }
}

#[derive(Clone, Debug)]
pub struct QwenCloudCatalogSeed {
    profile: VersionedProfileId,
    provider: ProviderId,
    account: AccountId,
    models: Vec<QwenCloudCatalogModel>,
}

impl QwenCloudCatalogSeed {
    pub fn resolve(
        profile: VersionedProfileId,
        provider: ProviderId,
        account: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        if provider.as_str() != QWENCLOUD_PROVIDER {
            return Err(ModelServiceError::new(format!(
                "catalog profile {profile} requires ProviderId {QWENCLOUD_PROVIDER}"
            )));
        }
        let definition = CATALOGS
            .iter()
            .find(|definition| definition.profile == profile.as_str())
            .ok_or_else(|| {
                ModelServiceError::new(format!("unknown built-in catalog profile {profile}"))
            })?;
        if definition.rows.len() > MAX_CATALOG_ROWS {
            return Err(ModelServiceError::new(
                "built-in catalog contains more than 4096 model rows",
            ));
        }
        let endpoint = NormalizedEndpoint::parse(definition.endpoint)?;
        let provider_display_name =
            provider_display_name.or_else(|| Some(PROVIDER_DISPLAY_FALLBACK.to_owned()));
        let base_profile = profile_layer(DEFAULT_INPUT_LIMIT);
        let mut seen = HashSet::new();
        let mut models = Vec::with_capacity(definition.rows.len());
        for row in definition.rows {
            let model_id = ModelId::new(row.model)?;
            if !seen.insert(model_id.clone()) {
                return Err(ModelServiceError::new(format!(
                    "catalog profile {profile} repeats ModelId {model_id}"
                )));
            }
            models.push(QwenCloudCatalogModel::from_row(
                &provider,
                &account,
                provider_display_name.clone(),
                account_display_name.clone(),
                &endpoint,
                &base_profile,
                row,
                model_id,
            )?);
        }
        models.sort_by(|left, right| {
            crate::normalized_search_key(left.display_name())
                .cmp(&crate::normalized_search_key(right.display_name()))
                .then_with(|| left.model_id().cmp(right.model_id()))
        });
        Ok(Self {
            profile,
            provider,
            account,
            models,
        })
    }

    #[must_use]
    pub const fn profile(&self) -> &VersionedProfileId {
        &self.profile
    }

    #[must_use]
    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    #[must_use]
    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    #[must_use]
    pub fn models(&self) -> &[QwenCloudCatalogModel] {
        &self.models
    }

    #[must_use]
    pub fn model(&self, model: &ModelId) -> Option<&QwenCloudCatalogModel> {
        self.models
            .iter()
            .find(|candidate| candidate.model_id() == model)
    }
}

#[derive(Clone, Debug)]
pub struct QwenCloudCatalogModel {
    provider: ProviderId,
    account: AccountId,
    model_id: ModelId,
    entry: Option<ModelCatalogEntry>,
    display_name: String,
    input_limit: Option<u64>,
    output_limit: Option<u64>,
    tool_policy: Option<VersionedProfileId>,
    reasoning: Option<bool>,
    availability: QwenCloudCatalogAvailability,
}

impl QwenCloudCatalogModel {
    #[allow(clippy::too_many_arguments)]
    fn from_row(
        provider: &ProviderId,
        account: &AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
        endpoint: &NormalizedEndpoint,
        base_profile: &ModelProfileLayer,
        row: &CatalogRow,
        model_id: ModelId,
    ) -> Result<Self, ModelServiceError> {
        let availability = if !row.text_input {
            QwenCloudCatalogAvailability::Disabled(
                QwenCloudCatalogDisabledReason::TextInputUnsupported,
            )
        } else if !row.text_output {
            QwenCloudCatalogAvailability::Disabled(
                QwenCloudCatalogDisabledReason::TextOutputUnsupported,
            )
        } else {
            QwenCloudCatalogAvailability::Enabled
        };
        let tool_policy = row.text_output.then(|| {
            valid_profile(if row.tools {
                "local-tools/v1"
            } else {
                "no-tools/v1"
            })
        });
        let entry = match availability {
            QwenCloudCatalogAvailability::Enabled => {
                let override_profile = ModelProfileLayer::new(
                    None,
                    None,
                    (row.input_limit != DEFAULT_INPUT_LIMIT).then_some(row.input_limit),
                    (row.output_limit != DEFAULT_OUTPUT_LIMIT).then_some(row.output_limit),
                    None,
                    None,
                    tool_policy.clone(),
                    None,
                );
                let profile =
                    EffectiveModelProfile::resolve(Some(base_profile), &override_profile)?;
                let binding = EffectiveModelBinding::new(
                    provider.clone(),
                    account.clone(),
                    model_id.clone(),
                    profile.api_dialect(),
                    endpoint.clone(),
                );
                let entry = ModelCatalogEntry::with_explicit_profile(
                    binding,
                    provider_display_name,
                    account_display_name,
                    Some(row.model.to_owned()),
                    profile,
                )?;
                Some(entry)
            },
            QwenCloudCatalogAvailability::Disabled(_) => None,
        };
        Ok(Self {
            provider: provider.clone(),
            account: account.clone(),
            model_id,
            entry,
            display_name: row.model.to_owned(),
            input_limit: row.text_input.then_some(row.input_limit),
            output_limit: row.text_output.then_some(row.output_limit),
            tool_policy,
            reasoning: row.reasoning,
            availability,
        })
    }

    #[must_use]
    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    #[must_use]
    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    #[must_use]
    pub const fn model_id(&self) -> &ModelId {
        &self.model_id
    }

    #[must_use]
    pub const fn entry(&self) -> Option<&ModelCatalogEntry> {
        self.entry.as_ref()
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    #[must_use]
    pub const fn input_limit(&self) -> Option<u64> {
        self.input_limit
    }

    #[must_use]
    pub const fn output_limit(&self) -> Option<u64> {
        self.output_limit
    }

    #[must_use]
    pub fn tool_policy(&self) -> Option<&str> {
        self.tool_policy.as_ref().map(VersionedProfileId::as_str)
    }

    #[must_use]
    pub const fn reasoning(&self) -> Option<bool> {
        self.reasoning
    }

    #[must_use]
    pub const fn availability(&self) -> QwenCloudCatalogAvailability {
        self.availability
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        matches!(self.availability, QwenCloudCatalogAvailability::Enabled)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QwenCloudCatalogAvailability {
    Enabled,
    Disabled(QwenCloudCatalogDisabledReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QwenCloudCatalogDisabledReason {
    TextInputUnsupported,
    TextOutputUnsupported,
}

impl QwenCloudCatalogDisabledReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TextInputUnsupported => "text input unsupported",
            Self::TextOutputUnsupported => "text output unsupported",
        }
    }
}

fn profile_layer(input_limit: u64) -> ModelProfileLayer {
    ModelProfileLayer::new(
        Some(ApiDialect::OpenAiChatCompletions),
        Some(valid_profile("utf8-bytes/v1")),
        Some(input_limit),
        Some(DEFAULT_OUTPUT_LIMIT),
        Some(empty_parameters()),
        Some(empty_parameters()),
        Some(valid_profile("local-tools/v1")),
        Some(valid_profile("semantic-terminal/v1")),
    )
}

fn empty_parameters() -> ModelProfileParameters {
    serde_json::from_value(serde_json::json!({}))
        .expect("an empty object is a valid model profile parameter mapping")
}

fn valid_profile(value: &str) -> VersionedProfileId {
    VersionedProfileId::new(value).expect("the built-in profile identifier is valid")
}
