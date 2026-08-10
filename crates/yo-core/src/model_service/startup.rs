use super::{ModelCatalog, ModelSelection, ModelSelectionController, ModelServiceError};

/// One startup target. Host targets and remote model coordinates never share an identity space.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StartupTarget {
    HostCodex,
    Model(ModelSelection),
}

impl StartupTarget {
    pub const HOST_CODEX_REFERENCE: &'static str = "host:codex";

    #[must_use]
    pub const fn model(&self) -> Option<&ModelSelection> {
        match self {
            Self::HostCodex => None,
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
        .resolve_model(selection.provider(), selection.account(), selection.model())
        .map(|_| ())
}
