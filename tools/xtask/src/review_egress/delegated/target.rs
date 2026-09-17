use super::super::{
    model::{
        AuthorizedDelegatedTarget, AuthorizedDelegatedTargetV1Alpha2,
        AuthorizedDelegatedTargetV1Alpha3, DELEGATED_EXECUTION_PROFILE,
        DelegatedAuthorizationDocument, DelegatedRequest,
    },
    validation::compact_token,
};

pub(super) const MAX_HOST_TOKEN_BYTES: usize = 32;

pub(super) enum AuthorizedTarget<'a> {
    Alpha1(&'a AuthorizedDelegatedTarget),
    Alpha2(&'a AuthorizedDelegatedTargetV1Alpha2),
    Alpha3(&'a AuthorizedDelegatedTargetV1Alpha3),
}

impl AuthorizedTarget<'_> {
    pub(super) fn allow_original_fresh(&self) -> bool {
        match self {
            Self::Alpha1(value) => value.allow_original_fresh,
            Self::Alpha2(value) => value.max_original_fresh_requests == 1,
            Self::Alpha3(value) => value.max_original_fresh_requests == 1,
        }
    }

    pub(super) fn max_finding_resolution_resume_requests(&self) -> usize {
        match self {
            Self::Alpha1(value) => usize::from(value.allow_finding_resolution_resume),
            Self::Alpha2(value) => value.max_finding_resolution_resume_requests,
            Self::Alpha3(value) => value.max_finding_resolution_resume_requests,
        }
    }

    pub(super) fn max_packet_bytes(&self) -> usize {
        match self {
            Self::Alpha1(value) => value.max_packet_bytes,
            Self::Alpha2(value) => value.max_packet_bytes,
            Self::Alpha3(value) => value.max_packet_bytes,
        }
    }

    pub(super) fn max_managed_payload_tokens(&self) -> usize {
        match self {
            Self::Alpha1(value) => value.max_managed_payload_tokens,
            Self::Alpha2(value) => value.max_managed_payload_tokens,
            Self::Alpha3(value) => value.max_managed_payload_tokens,
        }
    }
}

pub(super) fn authorized_target<'a>(
    authorization: &'a DelegatedAuthorizationDocument,
    request: &DelegatedRequest,
) -> Result<AuthorizedTarget<'a>, String> {
    match authorization {
        DelegatedAuthorizationDocument::Alpha1(value) => value
            .targets
            .iter()
            .find(|target| {
                target.host == request.target.host()
                    && target.execution_profile == request.execution_profile
            })
            .map(AuthorizedTarget::Alpha1),
        DelegatedAuthorizationDocument::Alpha2(value) => value
            .targets
            .iter()
            .find(|target| {
                target.host == request.target.host()
                    && target.execution_profile == request.execution_profile
            })
            .map(AuthorizedTarget::Alpha2),
        DelegatedAuthorizationDocument::Alpha3(value) => value
            .targets
            .iter()
            .find(|target| {
                target.host == request.target.host()
                    && target.execution_profile == request.execution_profile
            })
            .map(AuthorizedTarget::Alpha3),
    }
    .ok_or_else(|| "requested delegated review target is not authorized".to_owned())
}
pub(super) fn validate_host(host: &str) -> Result<(), String> {
    compact_token(host, MAX_HOST_TOKEN_BYTES, "delegated host")?;
    if matches!(host, "codex" | "grok") {
        Ok(())
    } else {
        Err(format!(
            "unsupported delegated review host `{host}`; expected `codex` or `grok`"
        ))
    }
}

pub(super) fn require_execution_profile(profile: &str) -> Result<(), String> {
    if profile == DELEGATED_EXECUTION_PROFILE {
        Ok(())
    } else {
        Err(format!(
            "delegated review requires execution profile `{DELEGATED_EXECUTION_PROFILE}`"
        ))
    }
}
