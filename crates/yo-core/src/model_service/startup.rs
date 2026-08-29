use super::{ModelCatalog, ModelSelection, ModelSelectionController, ModelServiceError};

/// Stable identity of one delegated agent host, independent from model Provider coordinates.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HostId(String);

/// One startup target. Host targets and remote model coordinates never share an identity space.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StartupTarget {
    Host(HostId),
    Model(ModelSelection),
}

impl HostId {
    pub const CODEX: &'static str = "codex";
    pub const GROK: &'static str = "grok";

    pub fn new(value: impl Into<String>) -> Result<Self, ModelServiceError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && value
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && value
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric);
        if !valid {
            return Err(ModelServiceError::new(
                "Host ID must be 1-64 lowercase ASCII letters, digits, or interior hyphens",
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn codex() -> Self {
        Self(Self::CODEX.to_owned())
    }

    #[must_use]
    pub fn grok() -> Self {
        Self(Self::GROK.to_owned())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn reference(&self) -> String {
        format!("host:{}", self.0)
    }

    pub fn from_reference(reference: &str) -> Result<Option<Self>, ModelServiceError> {
        let Some(value) = reference.strip_prefix("host:") else {
            return Ok(None);
        };
        if value.contains(':') {
            return Ok(None);
        }
        Self::new(value).map(Some)
    }
}

impl StartupTarget {
    pub const HOST_CODEX_REFERENCE: &'static str = "host:codex";
    pub const HOST_GROK_REFERENCE: &'static str = "host:grok";

    #[must_use]
    pub fn host_codex() -> Self {
        Self::Host(HostId::codex())
    }

    #[must_use]
    pub fn host_grok() -> Self {
        Self::Host(HostId::grok())
    }

    #[must_use]
    pub fn host(&self) -> Option<&HostId> {
        match self {
            Self::Host(host) => Some(host),
            Self::Model(_) => None,
        }
    }

    #[must_use]
    pub const fn model(&self) -> Option<&ModelSelection> {
        match self {
            Self::Host(_) => None,
            Self::Model(selection) => Some(selection),
        }
    }
}

/// Immutable startup policy captured by the process host for one invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupPolicy {
    allow_user_override: bool,
    enforced_target: Option<StartupTarget>,
    default_target: Option<StartupTarget>,
}

impl StartupPolicy {
    pub fn new(
        allow_user_override: bool,
        enforced_target: Option<StartupTarget>,
        default_target: Option<StartupTarget>,
    ) -> Result<Self, ModelServiceError> {
        let valid = if allow_user_override {
            enforced_target.is_none()
        } else {
            enforced_target.is_some() && default_target.is_none()
        };
        if !valid {
            return Err(ModelServiceError::new(
                "startup policy must be overridable with no enforced target, or enforced with exactly one target and no default",
            ));
        }
        Ok(Self {
            allow_user_override,
            enforced_target,
            default_target,
        })
    }

    #[must_use]
    pub fn initial() -> Self {
        Self {
            allow_user_override: true,
            enforced_target: None,
            default_target: None,
        }
    }

    #[must_use]
    pub const fn default_target(&self) -> Option<&StartupTarget> {
        self.default_target.as_ref()
    }
}

/// The four independently captured startup selection layers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StartupSelectionSources<'a> {
    pub invocation: Option<&'a str>,
    pub stored_preference: Option<StartupTarget>,
    pub operator_target: Option<StartupTarget>,
}

/// Resolves startup precedence without inventing an implicit target.
pub fn resolve_startup_target(
    catalog: &ModelCatalog,
    policy: &StartupPolicy,
    sources: StartupSelectionSources<'_>,
) -> Result<Option<StartupTarget>, ModelServiceError> {
    validate_target(catalog, sources.stored_preference.as_ref())?;
    validate_target(catalog, policy.enforced_target.as_ref())?;
    validate_target(catalog, policy.default_target.as_ref())?;
    validate_target(catalog, sources.operator_target.as_ref())?;

    let namespace = if policy.allow_user_override {
        sources
            .stored_preference
            .as_ref()
            .or(policy.default_target.as_ref())
            .or(sources.operator_target.as_ref())
    } else {
        policy.enforced_target.as_ref()
    }
    .and_then(StartupTarget::model)
    .cloned();
    let invocation = sources
        .invocation
        .map(|reference| {
            ModelSelectionController::new(catalog.clone(), namespace)
                .resolve_target_reference(reference)
        })
        .transpose()?;

    if !policy.allow_user_override {
        let enforced = policy
            .enforced_target
            .clone()
            .expect("validated enforced startup policy has one target");
        if invocation
            .as_ref()
            .is_some_and(|target| target != &enforced)
        {
            return Err(ModelServiceError::new(
                "the invocation target conflicts with the enforced startup policy",
            ));
        }
        return Ok(Some(enforced));
    }

    Ok(invocation
        .or(sources.stored_preference)
        .or_else(|| policy.default_target.clone())
        .or(sources.operator_target))
}

fn validate_target(
    catalog: &ModelCatalog,
    target: Option<&StartupTarget>,
) -> Result<(), ModelServiceError> {
    let Some(StartupTarget::Model(selection)) = target else {
        return Ok(());
    };
    catalog
        .resolve_model(selection.provider(), selection.account(), selection.model())?
        .require_enabled()
}
