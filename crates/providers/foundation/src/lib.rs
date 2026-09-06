//! Provider-neutral composition contracts for managed model connectors.
//!
//! This crate owns route selection and construction only. Request execution
//! remains [`yo_core::ModelConnector`]'s responsibility, while catalog
//! discovery and account operations remain provider-specific control-plane work.

use std::{
    collections::{HashMap, hash_map::Entry},
    error::Error,
    fmt,
};

use yo_core::{
    ApiCredential, ApiDialect, ConnectorError, ConnectorId, EffectiveModelBinding,
    ModelCatalogEntry, ModelConnector, ModelConnectorLimits,
};

/// An exact, owned model-connector dispatch key.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ModelConnectorRoute {
    connector_id: ConnectorId,
    api_dialect: ApiDialect,
}

impl ModelConnectorRoute {
    /// Builds an exact route, rejecting Connector/dialect pairs that core does
    /// not admit as one managed connector identity.
    pub fn new(
        connector_id: ConnectorId,
        api_dialect: ApiDialect,
    ) -> Result<Self, ModelConnectorRouteError> {
        if connector_id != ConnectorId::for_dialect(api_dialect) {
            return Err(ModelConnectorRouteError::new(format!(
                "connector {} does not match API dialect {}",
                connector_id, api_dialect
            )));
        }
        Ok(Self {
            connector_id,
            api_dialect,
        })
    }

    #[must_use]
    pub fn from_binding(binding: &EffectiveModelBinding) -> Self {
        Self {
            connector_id: binding.connector_id().clone(),
            api_dialect: binding.api_dialect(),
        }
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
    pub fn matches(&self, binding: &EffectiveModelBinding) -> bool {
        self == &Self::from_binding(binding)
    }
}

impl fmt::Display for ModelConnectorRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} ({})", self.connector_id, self.api_dialect)
    }
}

/// A rejected Connector/dialect route pair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelConnectorRouteError {
    message: String,
}

impl ModelConnectorRouteError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ModelConnectorRouteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ModelConnectorRouteError {}

/// Build inputs for one already-selected managed connector.
///
/// The request consumes the credential and limits, while borrowing the catalog
/// entry whose route the registry has already selected.
pub struct ModelConnectorBuildRequest<'a> {
    catalog_entry: &'a ModelCatalogEntry,
    credential: ApiCredential,
    limits: ModelConnectorLimits,
}

impl<'a> ModelConnectorBuildRequest<'a> {
    #[must_use]
    pub const fn new(
        catalog_entry: &'a ModelCatalogEntry,
        credential: ApiCredential,
        limits: ModelConnectorLimits,
    ) -> Self {
        Self {
            catalog_entry,
            credential,
            limits,
        }
    }

    #[must_use]
    pub const fn catalog_entry(&self) -> &ModelCatalogEntry {
        self.catalog_entry
    }

    #[must_use]
    pub fn into_parts(self) -> (&'a ModelCatalogEntry, ApiCredential, ModelConnectorLimits) {
        (self.catalog_entry, self.credential, self.limits)
    }
}

/// Constructs one connector for a single exact route.
///
/// The trait is object-safe so a composition root can keep heterogeneous
/// connector implementations without coupling the managed backend to any of
/// them.
pub trait ModelConnectorFactory: Send + Sync {
    fn route(&self) -> ModelConnectorRoute;

    fn build(
        &self,
        request: ModelConnectorBuildRequest<'_>,
    ) -> Result<Box<dyn ModelConnector>, ConnectorError>;
}

/// Exact-route lookup and construction for a finite composition-root factory set.
pub struct ModelConnectorFactoryRegistry {
    factories: HashMap<ModelConnectorRoute, Box<dyn ModelConnectorFactory>>,
}

impl ModelConnectorFactoryRegistry {
    pub fn new(
        factories: impl IntoIterator<Item = Box<dyn ModelConnectorFactory>>,
    ) -> Result<Self, ModelConnectorFactoryRegistryError> {
        let mut indexed = HashMap::new();
        for factory in factories {
            let route = factory.route();
            match indexed.entry(route) {
                Entry::Vacant(entry) => {
                    entry.insert(factory);
                },
                Entry::Occupied(entry) => {
                    return Err(ModelConnectorFactoryRegistryError::DuplicateRoute(
                        entry.key().clone(),
                    ));
                },
            }
        }
        Ok(Self { factories: indexed })
    }

    fn factory_for(
        &self,
        route: &ModelConnectorRoute,
    ) -> Result<&dyn ModelConnectorFactory, ModelConnectorFactoryRegistryError> {
        self.factories
            .get(route)
            .map(Box::as_ref)
            .ok_or_else(|| ModelConnectorFactoryRegistryError::MissingRoute(route.clone()))
    }

    pub fn build(
        &self,
        request: ModelConnectorBuildRequest<'_>,
    ) -> Result<Box<dyn ModelConnector>, ModelConnectorFactoryRegistryError> {
        let route = ModelConnectorRoute::from_binding(request.catalog_entry().binding());
        self.factory_for(&route)?
            .build(request)
            .map_err(|source| ModelConnectorFactoryRegistryError::Build { route, source })
    }
}

/// Errors limited to exact-route registration and dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelConnectorFactoryRegistryError {
    DuplicateRoute(ModelConnectorRoute),
    MissingRoute(ModelConnectorRoute),
    Build {
        route: ModelConnectorRoute,
        source: ConnectorError,
    },
}

impl fmt::Display for ModelConnectorFactoryRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateRoute(route) => {
                write!(formatter, "duplicate model connector route {route}")
            },
            Self::MissingRoute(route) => {
                write!(formatter, "no model connector factory for route {route}")
            },
            Self::Build { route, source } => {
                write!(
                    formatter,
                    "constructing model connector for route {route}: {source}"
                )
            },
        }
    }
}

impl Error for ModelConnectorFactoryRegistryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Build { source, .. } => Some(source),
            Self::DuplicateRoute(_) | Self::MissingRoute(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ErrorFactory {
        route: ModelConnectorRoute,
        error: ConnectorError,
    }

    impl ModelConnectorFactory for ErrorFactory {
        fn route(&self) -> ModelConnectorRoute {
            self.route.clone()
        }

        fn build(
            &self,
            _: ModelConnectorBuildRequest<'_>,
        ) -> Result<Box<dyn ModelConnector>, ConnectorError> {
            Err(self.error.clone())
        }
    }

    struct SuccessFactory(ModelConnectorRoute);

    impl ModelConnectorFactory for SuccessFactory {
        fn route(&self) -> ModelConnectorRoute {
            self.0.clone()
        }

        fn build(
            &self,
            _: ModelConnectorBuildRequest<'_>,
        ) -> Result<Box<dyn ModelConnector>, ConnectorError> {
            Ok(Box::new(Connector))
        }
    }

    struct Connector;

    impl ModelConnector for Connector {
        fn request_url(&self) -> &str {
            "https://factory.test"
        }

        fn tokenization_payload(
            &self,
            _: &yo_core::ModelConnectorRequest,
        ) -> Result<serde_json::Value, ConnectorError> {
            Ok(serde_json::Value::Null)
        }

        fn start(
            &self,
            _: yo_core::ModelConnectorRequest,
            _: yo_core::ModelConnectorCancellation,
        ) -> Result<Box<dyn yo_core::ModelConnectorStreamPort>, ConnectorError> {
            Err(ConnectorError::new(
                yo_core::ConnectorFailureKind::Configuration,
                "test connector does not start streams",
            ))
        }
    }

    fn route(api_dialect: ApiDialect) -> ModelConnectorRoute {
        ModelConnectorRoute::new(ConnectorId::for_dialect(api_dialect), api_dialect).unwrap()
    }

    fn catalog_entry(api_dialect: ApiDialect) -> ModelCatalogEntry {
        let binding = EffectiveModelBinding::new(
            yo_core::ProviderId::new("provider").unwrap(),
            yo_core::AccountId::new("account").unwrap(),
            yo_core::ModelId::new("model").unwrap(),
            api_dialect,
            yo_core::NormalizedEndpoint::parse("https://example.test/v1").unwrap(),
        );
        ModelCatalogEntry::new(
            binding,
            None,
            None,
            None,
            yo_core::ModelContextProfile::new(1_024, 256, "test-tokenizer/v1").unwrap(),
        )
        .unwrap()
    }

    fn build_request(entry: &ModelCatalogEntry) -> ModelConnectorBuildRequest<'_> {
        ModelConnectorBuildRequest::new(
            entry,
            ApiCredential::new("test-credential").unwrap(),
            ModelConnectorLimits::default(),
        )
    }

    fn error_factory(route: ModelConnectorRoute) -> Box<dyn ModelConnectorFactory> {
        Box::new(ErrorFactory {
            route,
            error: ConnectorError::new(
                yo_core::ConnectorFailureKind::Configuration,
                "test factory build failure",
            ),
        })
    }

    // route는 같은 Provider 이름이 아니라 closed Connector/dialect 쌍으로만 binding을 고릅니다.
    #[test]
    fn route_is_an_exact_connector_and_dialect_match() {
        let responses_route = route(ApiDialect::OpenAiResponses);
        assert_eq!(
            responses_route.connector_id().as_str(),
            ConnectorId::OPENAI_RESPONSES
        );
        assert_eq!(responses_route.api_dialect(), ApiDialect::OpenAiResponses);
        let binding = EffectiveModelBinding::new(
            yo_core::ProviderId::new("provider").unwrap(),
            yo_core::AccountId::new("account").unwrap(),
            yo_core::ModelId::new("model").unwrap(),
            ApiDialect::OpenAiResponses,
            yo_core::NormalizedEndpoint::parse("https://example.test/v1").unwrap(),
        );
        assert!(responses_route.matches(&binding));
        assert!(!route(ApiDialect::OpenAiChatCompletions).matches(&binding));
        assert!(
            ModelConnectorRoute::new(
                ConnectorId::for_dialect(ApiDialect::OpenAiChatCompletions),
                ApiDialect::OpenAiResponses,
            )
            .is_err()
        );
    }

    // 두 factory가 같은 exact route를 claim하면 composition 전에 모호성을 닫습니다.
    #[test]
    fn registry_rejects_duplicate_exact_routes() {
        let route = route(ApiDialect::OpenAiResponses);
        let result = ModelConnectorFactoryRegistry::new([
            error_factory(route.clone()),
            error_factory(route.clone()),
        ]);
        let Err(error) = result else {
            panic!("duplicate routes must be rejected");
        };

        assert_eq!(
            error,
            ModelConnectorFactoryRegistryError::DuplicateRoute(route)
        );
    }

    // 등록하지 않은 route는 fallback 없이 명시적인 missing-route 진단으로 끝납니다.
    #[test]
    fn registry_reports_a_missing_exact_route() {
        let missing = route(ApiDialect::KimiChatCompletions);
        let entry = catalog_entry(ApiDialect::KimiChatCompletions);
        let registry =
            ModelConnectorFactoryRegistry::new([error_factory(route(ApiDialect::OpenAiResponses))])
                .unwrap();

        let result = registry.build(build_request(&entry));
        let Err(error) = result else {
            panic!("unregistered route must be rejected");
        };
        assert_eq!(
            error,
            ModelConnectorFactoryRegistryError::MissingRoute(missing)
        );
    }

    // exact route가 등록되어 있으면 registry가 그 factory의 constructed connector를 반환합니다.
    #[test]
    fn registry_dispatches_an_exact_matching_route() {
        let entry = catalog_entry(ApiDialect::OpenAiResponses);
        let registry = ModelConnectorFactoryRegistry::new([Box::new(SuccessFactory(route(
            ApiDialect::OpenAiResponses,
        )))
            as Box<dyn ModelConnectorFactory>])
        .unwrap();

        let connector = registry.build(build_request(&entry)).unwrap();
        assert_eq!(connector.request_url(), "https://factory.test");
    }

    // factory failure는 selected exact route와 original typed ConnectorError를 함께 보존합니다.
    #[test]
    fn registry_preserves_the_build_error_route_and_source() {
        let route = route(ApiDialect::OpenAiResponses);
        let entry = catalog_entry(ApiDialect::OpenAiResponses);
        let registry = ModelConnectorFactoryRegistry::new([error_factory(route.clone())]).unwrap();

        let result = registry.build(build_request(&entry));
        let Err(error) = result else {
            panic!("failing factory must preserve its build error");
        };
        assert_eq!(
            error,
            ModelConnectorFactoryRegistryError::Build {
                route,
                source: ConnectorError::new(
                    yo_core::ConnectorFailureKind::Configuration,
                    "test factory build failure",
                ),
            }
        );
    }
}
