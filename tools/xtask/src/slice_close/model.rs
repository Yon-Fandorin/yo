use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const SCHEMA: &str = "yo.slice-close-plan/v1";

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
    pub(super) effects: Effects,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Effects {
    pub(super) remove_worktree: bool,
    pub(super) remove_binding: bool,
    pub(super) delete_slice_branch: bool,
}

impl Effects {
    pub(super) fn all() -> Self {
        Self {
            remove_worktree: true,
            remove_binding: true,
            delete_slice_branch: true,
        }
    }
}

#[derive(Serialize)]
struct PlanIdentity<'a> {
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
    if plan.schema != SCHEMA {
        return Err(format!(
            "unsupported Slice close plan schema `{}`; expected `{SCHEMA}`",
            plan.schema
        ));
    }
    validate_slice_name(&plan.slice)?;
    if plan.effects != Effects::all() {
        return Err("Slice close plan must declare all three bounded cleanup effects".to_owned());
    }
    if slice_ref_for(&plan.integration_ref, &plan.slice)? != plan.slice_ref {
        return Err("Slice close plan branch does not match its Slice name".to_owned());
    }
    Ok(())
}

pub(super) fn identity(plan: &Plan) -> Result<String, String> {
    let identity = PlanIdentity {
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
    let bytes = serde_json::to_vec(&identity)
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
