use std::{collections::BTreeSet, path::Path};

use serde::Deserialize;

use super::{
    capture::{capture_authorities, parse_activation_request},
    model::{DELIVERY_PROFILE_V1_ALPHA3, REQUEST_SCHEMA_V1_ALPHA3},
    trusted_git::trusted_git_bytes,
};
use crate::review_protocol::Captured;

pub(super) const CAPABILITY_PATH: &str = "tools/xtask/src/review/packet/v1alpha3-capability.json";
pub(super) const CAPABILITY_BYTES: &[u8] = include_bytes!("v1alpha3-capability.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capability {
    schema: String,
    request_schema: String,
    delivery_profile: String,
    active_record_path: String,
    checkpoint_directory: String,
    registered_manifest_paths: Vec<String>,
}

pub(super) fn require_prospective_activation_boundary(
    repository: &Path,
    trusted_commit: &str,
    candidate_commit: &str,
    activation_request: &Captured,
) -> Result<(), String> {
    let trusted_capability = capture_authorities(
        repository,
        trusted_commit,
        &[CAPABILITY_PATH.to_owned()],
    )
    .map_err(|_| {
        "prospective activation review is not enabled by trusted develop; use ordinary review"
            .to_owned()
    })?
    .pop()
    .expect("one capability path was requested");
    if trusted_capability.bytes != CAPABILITY_BYTES {
        return Err(
            "trusted develop does not contain this exact prospective activation review capability; use ordinary review"
                .to_owned(),
        );
    }

    let capability: Capability = serde_json::from_slice(CAPABILITY_BYTES)
        .map_err(|error| format!("invalid compiled prospective review capability: {error}"))?;
    if capability.schema != "yo.prospective-activation-review-capability/v1alpha1"
        || capability.request_schema != REQUEST_SCHEMA_V1_ALPHA3
        || capability.delivery_profile != DELIVERY_PROFILE_V1_ALPHA3
        || capability.active_record_path != "methexis/active-checkpoint.yaml"
        || capability.checkpoint_directory != "methexis/checkpoints"
        || capability.registered_manifest_paths
            != [
                "tools/methexis/examples/context-contract/manifest.json",
                "tools/methexis/examples/context-contract/stable-leaf-manifest.json",
            ]
    {
        return Err(
            "compiled prospective review capability is not the supported closed boundary"
                .to_owned(),
        );
    }

    let activation = parse_activation_request(&activation_request.bytes)?;
    let checkpoint_name = activation
        .checkpoint_id
        .strip_prefix("sha256:")
        .expect("validated activation CheckpointId");
    let mut expected_paths = BTreeSet::from([
        capability.active_record_path,
        format!("{}/{checkpoint_name}.yaml", capability.checkpoint_directory),
    ]);
    expected_paths.extend(capability.registered_manifest_paths);

    let changed = trusted_git_bytes(
        repository,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            trusted_commit,
            candidate_commit,
            "--",
        ],
    )?;
    if !changed.is_empty() && changed.last() != Some(&0) {
        return Err("trusted Git returned an unterminated activation path list".to_owned());
    }
    let actual_paths = changed
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            std::str::from_utf8(path)
                .map(str::to_owned)
                .map_err(|error| format!("activation candidate path is not UTF-8: {error}"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if actual_paths != expected_paths {
        return Err(
            "prospective review requires a later activation-only candidate; implementation, workflow, or unrelated path changes must use ordinary review"
                .to_owned(),
        );
    }
    Ok(())
}
