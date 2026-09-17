use std::{
    collections,
    path::{Path, PathBuf},
};

use super::{
    super::{
        model::{
            DELEGATED_AUTHORIZATION_SCHEMA, DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2,
            DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3, DELEGATED_REVIEW_CHAIN_PROFILE,
            DelegatedAuthorizationDocument, DelegatedRequest, ReviewKind, Session,
        },
        validation::{MAX_AUTHORIZED_TOKENS, PACKET_LIMIT, compact_token},
    },
    target,
};
use crate::git;

const MAX_HOST_TARGETS: usize = 2;
const MAX_FINDING_RESOLUTION_REQUESTS: usize = 63;

pub(super) fn canonical_authorization_path(repository: &Path) -> Result<PathBuf, String> {
    let common = git::trusted_output_in(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common = PathBuf::from(common.trim());
    if common.file_name().and_then(|name| name.to_str()) != Some(".git") {
        return Err(
            "trusted Git common directory must be the repository `.git` directory".to_owned(),
        );
    }
    let root = common
        .parent()
        .ok_or_else(|| "trusted Git common directory has no repository parent".to_owned())?;
    Ok(root
        .join(".local-exclude")
        .join("authorizations")
        .join("external-review-delegated.json"))
}

pub(super) fn validate_authorization(
    authorization: &DelegatedAuthorizationDocument,
) -> Result<(), String> {
    let (schema, status, target_count) = match authorization {
        DelegatedAuthorizationDocument::Alpha1(value) => {
            if value.schema != DELEGATED_AUTHORIZATION_SCHEMA {
                return Err(format!(
                    "unsupported delegated external review authorization schema `{}`; expected `{DELEGATED_AUTHORIZATION_SCHEMA}`, `{DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2}`, or `{DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3}`",
                    value.schema
                ));
            }
            (
                value.schema.as_str(),
                value.status.as_str(),
                value.targets.len(),
            )
        },
        DelegatedAuthorizationDocument::Alpha2(value) => {
            if value.schema != DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2 {
                return Err(format!(
                    "unsupported delegated external review authorization schema `{}`; expected `{DELEGATED_AUTHORIZATION_SCHEMA}`, `{DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2}`, or `{DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3}`",
                    value.schema
                ));
            }
            (
                value.schema.as_str(),
                value.status.as_str(),
                value.targets.len(),
            )
        },
        DelegatedAuthorizationDocument::Alpha3(value) => {
            if value.schema != DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3 {
                return Err(format!(
                    "unsupported delegated external review authorization schema `{}`; expected `{DELEGATED_AUTHORIZATION_SCHEMA}`, `{DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2}`, or `{DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3}`",
                    value.schema
                ));
            }
            if value.review_chain_profile != DELEGATED_REVIEW_CHAIN_PROFILE {
                return Err(format!(
                    "delegated authorization v1alpha3 requires review_chain_profile `{DELEGATED_REVIEW_CHAIN_PROFILE}`"
                ));
            }
            (
                value.schema.as_str(),
                value.status.as_str(),
                value.targets.len(),
            )
        },
    };
    if status != "active" {
        return Err("delegated external review authorization is not active".to_owned());
    }
    let Some(owner) = authorization.authority().strip_prefix("human/") else {
        return Err("delegated external review authority must start with `human/`".to_owned());
    };
    compact_token(owner, 122, "authorization human owner")?;
    compact_token(authorization.authority(), 128, "authorization authority")?;
    if target_count == 0 || target_count > MAX_HOST_TARGETS {
        return Err(format!(
            "delegated external review authorization requires 1..={MAX_HOST_TARGETS} targets"
        ));
    }
    let mut hosts = collections::BTreeSet::new();
    match authorization {
        DelegatedAuthorizationDocument::Alpha1(value) => {
            for target in &value.targets {
                validate_target_limits(
                    &mut hosts,
                    &target.host,
                    &target.execution_profile,
                    target.max_packet_bytes,
                    target.max_managed_payload_tokens,
                )?;
                if !target.allow_original_fresh && !target.allow_finding_resolution_resume {
                    return Err(
                        "an authorized delegated target must allow at least one review request kind"
                            .to_owned(),
                    );
                }
            }
        },
        DelegatedAuthorizationDocument::Alpha2(value) => {
            for target in &value.targets {
                validate_target_limits(
                    &mut hosts,
                    &target.host,
                    &target.execution_profile,
                    target.max_packet_bytes,
                    target.max_managed_payload_tokens,
                )?;
                if target.max_original_fresh_requests > 1
                    || target.max_finding_resolution_resume_requests > 1
                    || target.max_total_requests
                        != target.max_original_fresh_requests
                            + target.max_finding_resolution_resume_requests
                    || target.max_total_requests == 0
                {
                    return Err(
                        "delegated authorization v1alpha2 requires explicit 0..=1 original and finding-resolution limits whose nonzero sum equals max_total_requests"
                            .to_owned(),
                    );
                }
            }
        },
        DelegatedAuthorizationDocument::Alpha3(value) => {
            for target in &value.targets {
                validate_target_limits(
                    &mut hosts,
                    &target.host,
                    &target.execution_profile,
                    target.max_packet_bytes,
                    target.max_managed_payload_tokens,
                )?;
                if target.max_original_fresh_requests > 1
                    || target.max_finding_resolution_resume_requests
                        > MAX_FINDING_RESOLUTION_REQUESTS
                    || target.max_total_requests
                        != target.max_original_fresh_requests
                            + target.max_finding_resolution_resume_requests
                    || target.max_total_requests == 0
                {
                    return Err(format!(
                        "delegated authorization v1alpha3 requires original 0..=1 and finding-resolution 0..={MAX_FINDING_RESOLUTION_REQUESTS} limits whose nonzero sum equals max_total_requests"
                    ));
                }
            }
        },
    }
    debug_assert!(matches!(
        schema,
        DELEGATED_AUTHORIZATION_SCHEMA
            | DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2
            | DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3
    ));
    Ok(())
}
fn validate_target_limits<'a>(
    hosts: &mut collections::BTreeSet<&'a str>,
    host: &'a str,
    execution_profile: &str,
    max_packet_bytes: usize,
    max_managed_payload_tokens: usize,
) -> Result<(), String> {
    target::validate_host(host)?;
    target::require_execution_profile(execution_profile)?;
    if !hosts.insert(host) {
        return Err("delegated external review targets must be unique".to_owned());
    }
    if max_packet_bytes == 0 || max_packet_bytes > PACKET_LIMIT {
        return Err(format!(
            "authorized max_packet_bytes must be within 1..={PACKET_LIMIT}"
        ));
    }
    if max_managed_payload_tokens == 0 || max_managed_payload_tokens > MAX_AUTHORIZED_TOKENS {
        return Err(format!(
            "authorized max_managed_payload_tokens must be within 1..={MAX_AUTHORIZED_TOKENS}"
        ));
    }
    Ok(())
}

pub(super) fn authorize(
    request: &DelegatedRequest,
    authorization: &DelegatedAuthorizationDocument,
    review_kind: ReviewKind,
    finding_resolution_request_index: usize,
    packet_bytes: usize,
    managed_payload_tokens: usize,
) -> Result<(), String> {
    let target = target::authorized_target(authorization, request)?;
    match (review_kind, &request.session) {
        (ReviewKind::Original, Session::Fresh) if target.allow_original_fresh() => {},
        (ReviewKind::FindingResolution, Session::Resume { .. })
            if finding_resolution_request_index
                <= target.max_finding_resolution_resume_requests() => {},
        (ReviewKind::Original, Session::Fresh) => {
            return Err("the target does not authorize an original fresh review".to_owned());
        },
        (ReviewKind::FindingResolution, Session::Resume { .. }) => {
            return Err("the target does not authorize a finding-resolution resume".to_owned());
        },
        (ReviewKind::Original, Session::Resume { .. }) => {
            return Err("an original review requires a fresh Session".to_owned());
        },
        (ReviewKind::FindingResolution, Session::Fresh) => {
            return Err(
                "a finding-resolution review requires the existing reviewer Session".to_owned(),
            );
        },
    }
    if packet_bytes > target.max_packet_bytes() {
        return Err(format!(
            "review packet has {packet_bytes} bytes, exceeding the authorized {}-byte target limit",
            target.max_packet_bytes()
        ));
    }
    if managed_payload_tokens > target.max_managed_payload_tokens() {
        return Err(format!(
            "review packet has {managed_payload_tokens} managed tokens, exceeding the authorized {}-token target limit",
            target.max_managed_payload_tokens()
        ));
    }
    Ok(())
}
