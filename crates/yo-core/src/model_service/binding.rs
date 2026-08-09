use std::{fmt, str::FromStr};

use url::Url;

use super::{AccountId, ModelId, ModelServiceError, ProviderId};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ApiDialect {
    OpenAiResponses,
    OpenAiChatCompletions,
}

impl ApiDialect {
    pub const OPENAI_RESPONSES: &'static str = "openai-responses";
    pub const OPENAI_CHAT_COMPLETIONS: &'static str = "openai-chat-completions";

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiResponses => Self::OPENAI_RESPONSES,
            Self::OpenAiChatCompletions => Self::OPENAI_CHAT_COMPLETIONS,
        }
    }
}

impl fmt::Display for ApiDialect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ApiDialect {
    type Err = ModelServiceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            Self::OPENAI_RESPONSES => Ok(Self::OpenAiResponses),
            Self::OPENAI_CHAT_COMPLETIONS => Ok(Self::OpenAiChatCompletions),
            _ => Err(ModelServiceError::new(format!(
                "unsupported api_dialect {value:?}; expected {} or {}",
                Self::OPENAI_RESPONSES,
                Self::OPENAI_CHAT_COMPLETIONS,
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ConnectorId(String);

impl ConnectorId {
    pub const OPENAI_RESPONSES: &'static str = "openai-responses";
    pub const OPENAI_CHAT_COMPLETIONS: &'static str = "openai-chat-completions";

    pub fn new(value: impl Into<String>) -> Result<Self, ModelServiceError> {
        let value = value.into();
        if !matches!(
            value.as_str(),
            Self::OPENAI_RESPONSES | Self::OPENAI_CHAT_COMPLETIONS
        ) {
            return Err(ModelServiceError::new(format!(
                "unsupported Model Connector {value:?}; expected {} or {}",
                Self::OPENAI_RESPONSES,
                Self::OPENAI_CHAT_COMPLETIONS,
            )));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn for_dialect(dialect: ApiDialect) -> Self {
        let value = match dialect {
            ApiDialect::OpenAiResponses => Self::OPENAI_RESPONSES,
            ApiDialect::OpenAiChatCompletions => Self::OPENAI_CHAT_COMPLETIONS,
        };
        Self(value.to_owned())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ConnectorId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for ConnectorId {
    type Err = ModelServiceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct NormalizedEndpoint(Url);

impl NormalizedEndpoint {
    pub fn parse(value: &str) -> Result<Self, ModelServiceError> {
        let mut endpoint = Url::parse(value).map_err(|error| {
            ModelServiceError::new(format!("invalid model-service base_url: {error}"))
        })?;
        if endpoint.scheme() != "https" || endpoint.host_str().is_none() {
            return Err(ModelServiceError::new(
                "model-service base_url must be an absolute HTTPS URL",
            ));
        }
        if !endpoint.username().is_empty() || endpoint.password().is_some() {
            return Err(ModelServiceError::new(
                "model-service base_url must not contain user information",
            ));
        }
        if endpoint.query().is_some() || endpoint.fragment().is_some() {
            return Err(ModelServiceError::new(
                "model-service base_url must not contain a query or fragment",
            ));
        }
        let trimmed = endpoint.path().trim_end_matches('/').to_owned();
        endpoint.set_path(if trimmed.is_empty() { "/" } else { &trimmed });
        Ok(Self(endpoint))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub(crate) fn append_path_segment(&self, segment: &str) -> Result<Url, ModelServiceError> {
        if segment.is_empty() || segment.contains('/') || segment.chars().any(char::is_control) {
            return Err(ModelServiceError::new(
                "model-service endpoint path segment must be non-empty and contain no slash or control character",
            ));
        }
        let mut endpoint = self.0.clone();
        endpoint
            .path_segments_mut()
            .map_err(|_| ModelServiceError::new("model-service base_url cannot accept a path"))?
            .push(segment);
        Ok(endpoint)
    }

    pub(crate) fn append_path_segments(&self, segments: &[&str]) -> Result<Url, ModelServiceError> {
        let mut endpoint = self.0.clone();
        let mut path = endpoint
            .path_segments_mut()
            .map_err(|_| ModelServiceError::new("model-service base_url cannot accept a path"))?;
        for segment in segments {
            if segment.is_empty() || segment.contains('/') || segment.chars().any(char::is_control)
            {
                return Err(ModelServiceError::new(
                    "model-service endpoint path segment must be non-empty and contain no slash or control character",
                ));
            }
            path.push(segment);
        }
        drop(path);
        Ok(endpoint)
    }
}

impl fmt::Display for NormalizedEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct EffectiveModelBinding {
    provider_id: ProviderId,
    account_id: AccountId,
    model_id: ModelId,
    connector_id: ConnectorId,
    api_dialect: ApiDialect,
    endpoint: NormalizedEndpoint,
}

impl EffectiveModelBinding {
    #[must_use]
    pub fn new(
        provider_id: ProviderId,
        account_id: AccountId,
        model_id: ModelId,
        api_dialect: ApiDialect,
        endpoint: NormalizedEndpoint,
    ) -> Self {
        Self {
            provider_id,
            account_id,
            model_id,
            connector_id: ConnectorId::for_dialect(api_dialect),
            api_dialect,
            endpoint,
        }
    }

    pub fn from_durable(
        provider_id: ProviderId,
        account_id: AccountId,
        model_id: ModelId,
        connector_id: ConnectorId,
        api_dialect: ApiDialect,
        endpoint: NormalizedEndpoint,
    ) -> Result<Self, ModelServiceError> {
        let expected = ConnectorId::for_dialect(api_dialect);
        if connector_id != expected {
            return Err(ModelServiceError::new(format!(
                "durable connector {} does not match api_dialect {}",
                connector_id, api_dialect
            )));
        }
        Ok(Self {
            provider_id,
            account_id,
            model_id,
            connector_id,
            api_dialect,
            endpoint,
        })
    }

    #[must_use]
    pub const fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    #[must_use]
    pub const fn account_id(&self) -> &AccountId {
        &self.account_id
    }

    #[must_use]
    pub const fn model_id(&self) -> &ModelId {
        &self.model_id
    }

    #[must_use]
    pub const fn connector_id(&self) -> &ConnectorId {
        &self.connector_id
    }

    #[must_use]
    pub const fn api_dialect(&self) -> ApiDialect {
        self.api_dialect
    }

    #[must_use]
    pub const fn endpoint(&self) -> &NormalizedEndpoint {
        &self.endpoint
    }
}
