use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const SCHEMA: &str = "yo.slice-close-plan/v1alpha1";
const LEGACY_SCHEMA_V4: &str = "yo.slice-close-plan/v4";
const LEGACY_SCHEMA_V3: &str = "yo.slice-close-plan/v3";
const LEGACY_SCHEMA_V2: &str = "yo.slice-close-plan/v2";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Plan {
    pub(super) schema: String,
    pub(super) plan_id: String,
    pub(super) slice: String,
    pub(super) integration_ref: String,
    pub(super) integration_head: String,
    pub(super) accepted_commit: String,
    pub(super) slice_ref: String,
    pub(super) slice_head: String,
    pub(super) slice_base: String,
    pub(super) patch_id: String,
    pub(super) worktree_path: PathBuf,
    pub(super) binding_path: PathBuf,
    pub(super) contract_path: PathBuf,
    pub(super) contract_id: String,
    #[serde(default)]
    pub(super) retained_coordination_paths: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) coordination_cleanup_paths: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) close_metrics: Option<CloseMetricsArtifact>,
    pub(super) effects: Effects,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CloseMetricsArtifact {
    pub(super) path: PathBuf,
    pub(super) hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Effects {
    pub(super) remove_worktree: bool,
    pub(super) remove_binding: bool,
    pub(super) remove_coordination_contract: bool,
    pub(super) delete_slice_branch: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(super) remove_coordination_directory: bool,
}

impl Effects {
    pub(super) fn new(remove_coordination_contract: bool) -> Self {
        Self {
            remove_worktree: true,
            remove_binding: true,
            remove_coordination_contract,
            delete_slice_branch: true,
            remove_coordination_directory: remove_coordination_contract,
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Serialize)]
struct PlanIdentityV3<'a> {
    #[serde(flatten)]
    legacy: LegacyPlanIdentityV2<'a>,
    retained_coordination_paths: &'a [PathBuf],
}

#[derive(Serialize)]
struct PlanIdentityV4<'a> {
    #[serde(flatten)]
    legacy: PlanIdentityV3<'a>,
    close_metrics: &'a CloseMetricsArtifact,
}

#[derive(Serialize)]
struct PlanIdentityV1Alpha1<'a> {
    #[serde(flatten)]
    legacy: PlanIdentityV4<'a>,
    coordination_cleanup_paths: &'a [PathBuf],
}

#[derive(Serialize)]
struct LegacyPlanIdentityV2<'a> {
    schema: &'a str,
    slice: &'a str,
    integration_ref: &'a str,
    integration_head: &'a str,
    accepted_commit: &'a str,
    slice_ref: &'a str,
    slice_head: &'a str,
    slice_base: &'a str,
    patch_id: &'a str,
    worktree_path: &'a Path,
    binding_path: &'a Path,
    contract_path: &'a Path,
    contract_id: &'a str,
    effects: &'a Effects,
}

pub(super) fn validate_plan_shape(plan: &Plan) -> Result<(), String> {
    if !matches!(
        plan.schema.as_str(),
        SCHEMA | LEGACY_SCHEMA_V4 | LEGACY_SCHEMA_V3 | LEGACY_SCHEMA_V2
    ) {
        return Err(format!(
            "unsupported Slice close plan schema `{}`; expected `{SCHEMA}`, `{LEGACY_SCHEMA_V4}`, `{LEGACY_SCHEMA_V3}`, or `{LEGACY_SCHEMA_V2}`",
            plan.schema
        ));
    }
    if plan.schema == LEGACY_SCHEMA_V2
        && (!plan.retained_coordination_paths.is_empty()
            || !plan.coordination_cleanup_paths.is_empty())
    {
        return Err("legacy Slice close plans cannot contain coordination path sets".to_owned());
    }
    if matches!(plan.schema.as_str(), SCHEMA | LEGACY_SCHEMA_V4) && plan.close_metrics.is_none() {
        return Err("Slice close plan v1alpha1 or v4 requires bound close metrics".to_owned());
    }
    if matches!(plan.schema.as_str(), LEGACY_SCHEMA_V3 | LEGACY_SCHEMA_V2)
        && plan.close_metrics.is_some()
    {
        return Err("Slice close plans before v4 cannot contain close metrics".to_owned());
    }
    if plan.schema != SCHEMA
        && (!plan.coordination_cleanup_paths.is_empty()
            || plan.effects.remove_coordination_directory)
    {
        return Err(
            "stable Slice close plans through v4 cannot remove the coordination directory"
                .to_owned(),
        );
    }
    if plan.schema == SCHEMA
        && (plan.effects.remove_coordination_directory != plan.effects.remove_coordination_contract
            || (plan.effects.remove_coordination_directory
                && !plan.retained_coordination_paths.is_empty())
            || (!plan.effects.remove_coordination_directory
                && !plan.coordination_cleanup_paths.is_empty()))
    {
        return Err(
            "Slice close plan v1alpha1 must delete standard coordination as one effect or preserve nonstandard coordination"
                .to_owned(),
        );
    }
    validate_slice_name(&plan.slice)?;
    if !plan.effects.remove_worktree
        || !plan.effects.remove_binding
        || !plan.effects.delete_slice_branch
    {
        return Err("Slice close plan must declare every Git cleanup effect".to_owned());
    }
    if slice_ref_for(&plan.integration_ref, &plan.slice)? != plan.slice_ref {
        return Err("Slice close plan branch does not match its Slice name".to_owned());
    }
    Ok(())
}

pub(super) fn binds_retained_coordination(plan: &Plan) -> bool {
    matches!(plan.schema.as_str(), LEGACY_SCHEMA_V4 | LEGACY_SCHEMA_V3)
        || (plan.schema == SCHEMA && !plan.effects.remove_coordination_directory)
}

pub(super) fn binds_coordination_cleanup(plan: &Plan) -> bool {
    plan.schema == SCHEMA && plan.effects.remove_coordination_directory
}

pub(super) fn binds_close_metrics(plan: &Plan) -> bool {
    matches!(plan.schema.as_str(), SCHEMA | LEGACY_SCHEMA_V4)
}

pub(super) fn identity(plan: &Plan) -> Result<String, String> {
    let legacy = LegacyPlanIdentityV2 {
        schema: &plan.schema,
        slice: &plan.slice,
        integration_ref: &plan.integration_ref,
        integration_head: &plan.integration_head,
        accepted_commit: &plan.accepted_commit,
        slice_ref: &plan.slice_ref,
        slice_head: &plan.slice_head,
        slice_base: &plan.slice_base,
        patch_id: &plan.patch_id,
        worktree_path: &plan.worktree_path,
        binding_path: &plan.binding_path,
        contract_path: &plan.contract_path,
        contract_id: &plan.contract_id,
        effects: &plan.effects,
    };
    let v3 = PlanIdentityV3 {
        legacy,
        retained_coordination_paths: &plan.retained_coordination_paths,
    };
    let bytes = match plan.schema.as_str() {
        LEGACY_SCHEMA_V2 => serde_json::to_vec(&v3.legacy),
        LEGACY_SCHEMA_V3 => serde_json::to_vec(&v3),
        LEGACY_SCHEMA_V4 => {
            let close_metrics = plan
                .close_metrics
                .as_ref()
                .ok_or_else(|| "Slice close plan v4 requires bound close metrics".to_owned())?;
            serde_json::to_vec(&PlanIdentityV4 {
                legacy: v3,
                close_metrics,
            })
        },
        SCHEMA => {
            let close_metrics = plan.close_metrics.as_ref().ok_or_else(|| {
                "Slice close plan v1alpha1 requires bound close metrics".to_owned()
            })?;
            serde_json::to_vec(&PlanIdentityV1Alpha1 {
                legacy: PlanIdentityV4 {
                    legacy: v3,
                    close_metrics,
                },
                coordination_cleanup_paths: &plan.coordination_cleanup_paths,
            })
        },
        _ => {
            return Err(format!(
                "unsupported Slice close plan schema `{}`",
                plan.schema
            ));
        },
    }
    .map_err(|error| format!("cannot encode Slice close plan identity: {error}"))?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{digest}"))
}

pub(super) fn validate_slice_name(slice: &str) -> Result<(), String> {
    if slice.is_empty()
        || slice != slice.trim()
        || slice.contains('/')
        || matches!(slice, "." | "..")
    {
        return Err("Slice name must be one non-empty branch segment".to_owned());
    }
    Ok(())
}

pub(super) fn slice_ref_for(integration_ref: &str, slice: &str) -> Result<String, String> {
    validate_slice_name(slice)?;
    if integration_ref == "refs/heads/develop" {
        return Ok(format!("refs/heads/slice/direct/{slice}"));
    }
    let wave = integration_ref
        .strip_prefix("refs/heads/wave/")
        .filter(|wave| !wave.is_empty() && !wave.contains('/'))
        .ok_or_else(|| format!("unsupported integration ref `{integration_ref}`"))?;
    Ok(format!("refs/heads/slice/{wave}/{slice}"))
}
