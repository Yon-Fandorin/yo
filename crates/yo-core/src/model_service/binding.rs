use std::{fmt, str::FromStr};

use url::Url;

use super::{AccountId, ModelId, ModelServiceError, ProviderId};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ApiProtocol {
    OpenAiResponses,
}

impl ApiProtocol {
    pub const OPENAI_RESPONSES: &'static str = "openai-responses";

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiResponses => Self::OPENAI_RESPONSES,
        }
    }
}

impl fmt::Display for ApiProtocol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ApiProtocol {
    type Err = ModelServiceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            Self::OPENAI_RESPONSES => Ok(Self::OpenAiResponses),
            _ => Err(ModelServiceError::new(format!(
                "unsupported api_protocol {value:?}; expected {}",
                Self::OPENAI_RESPONSES
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ConnectorId(String);

impl ConnectorId {
    pub const OPENAI_RESPONSES: &'static str = "openai-responses";

    pub fn new(value: impl Into<String>) -> Result<Self, ModelServiceError> {
        let value = value.into();
        if value != Self::OPENAI_RESPONSES {
            return Err(ModelServiceError::new(format!(
                "unsupported Model Connector {value:?}; expected {}",
                Self::OPENAI_RESPONSES
            )));
        }
        Ok(Self(value))
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
    api_protocol: ApiProtocol,
    endpoint: NormalizedEndpoint,
}

impl EffectiveModelBinding {
    #[must_use]
    pub const fn new(
        provider_id: ProviderId,
        account_id: AccountId,
        model_id: ModelId,
        connector_id: ConnectorId,
        api_protocol: ApiProtocol,
        endpoint: NormalizedEndpoint,
    ) -> Self {
        Self {
            provider_id,
            account_id,
            model_id,
            connector_id,
            api_protocol,
            endpoint,
        }
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
    pub const fn api_protocol(&self) -> ApiProtocol {
        self.api_protocol
    }

    #[must_use]
    pub const fn endpoint(&self) -> &NormalizedEndpoint {
        &self.endpoint
    }
}
