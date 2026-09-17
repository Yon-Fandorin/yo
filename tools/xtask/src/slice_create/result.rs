use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::Serialize;

use super::{request, worktree};
use crate::{slice_contract, slice_worktree};

pub(super) const RESULT_SCHEMA: &str = "yo.slice-bootstrap/v1alpha1";

#[derive(Debug, Serialize)]
pub(super) struct ResultRecord {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) slice: String,
    pub(super) base: String,
    pub(super) base_ref: String,
    pub(super) integration_worktree: PathBuf,
    pub(super) branch_ref: String,
    pub(super) worktree_path: PathBuf,
    pub(super) contract_path: PathBuf,
    pub(super) binding_path: PathBuf,
    pub(super) effects: Effects,
    pub(super) next_action: NextAction,
}

#[derive(Debug, Serialize)]
pub(super) struct Effects {
    pub(super) contract: Effect,
    pub(super) branch: Effect,
    pub(super) worktree: Effect,
    pub(super) binding: Effect,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Effect {
    Created,
    Reused,
}

#[derive(Debug, Serialize)]
pub(super) struct NextAction {
    pub(super) cwd: PathBuf,
    pub(super) argv: [&'static str; 4],
}

pub(super) fn effect(created: bool) -> Effect {
    if created {
        Effect::Created
    } else {
        Effect::Reused
    }
}

pub(super) fn encode_failure(
    repository: &Path,
    bytes: Option<&[u8]>,
    error: String,
) -> Result<String, String> {
    let mut failure = serde_json::json!({
        "schema": RESULT_SCHEMA,
        "ok": false,
        "error": error,
        "effects": {
            "contract": {"state": "unknown"},
            "branch": {"state": "unknown"},
            "worktree": {"state": "unknown"},
            "binding": {"state": "unknown"}
        }
    });
    let Some(bytes) = bytes else {
        return serde_json::to_string_pretty(&failure)
            .map_err(|error| format!("cannot encode Slice bootstrap failure: {error}"));
    };
    let Ok(contract) = serde_json::from_slice::<slice_contract::SliceContract>(bytes) else {
        return serde_json::to_string_pretty(&failure)
            .map_err(|error| format!("cannot encode Slice bootstrap failure: {error}"));
    };
    failure["slice"] = serde_json::json!(contract.slice);
    failure["base"] = serde_json::json!(contract.base);
    failure["base_ref"] = serde_json::json!(contract.base_ref);
    let Ok(root) = slice_worktree::repository_root(repository) else {
        return serde_json::to_string_pretty(&failure)
            .map_err(|error| format!("cannot encode Slice bootstrap failure: {error}"));
    };
    let Ok(workspace) = slice_worktree::workspace_root(&root) else {
        return serde_json::to_string_pretty(&failure)
            .map_err(|error| format!("cannot encode Slice bootstrap failure: {error}"));
    };
    let contract_path = workspace
        .join(".local-exclude/coordination")
        .join(&contract.slice)
        .join("slice-contract.json");
    let worktree_path = workspace
        .join(".local-exclude/worktrees")
        .join(&contract.slice);
    let Ok(branch_ref) = worktree::branch_ref(&contract) else {
        return serde_json::to_string_pretty(&failure)
            .map_err(|error| format!("cannot encode Slice bootstrap failure: {error}"));
    };
    failure["branch_ref"] = serde_json::json!(branch_ref);
    failure["contract_path"] = serde_json::json!(contract_path);
    failure["worktree_path"] = serde_json::json!(worktree_path);
    let prepared_contract = match request::read_optional_contract(&contract_path) {
        Ok(Some(actual)) if actual == bytes => {
            failure["effects"]["contract"] = serde_json::json!({"state": "prepared"});
            true
        },
        Ok(Some(_)) => {
            failure["effects"]["contract"] = serde_json::json!({"state": "conflicting"});
            false
        },
        Ok(None) => {
            failure["effects"]["contract"] = serde_json::json!({"state": "absent"});
            false
        },
        Err(detail) => {
            failure["effects"]["contract"] =
                serde_json::json!({"state": "conflicting", "detail": detail});
            false
        },
    };
    let branch_prepared = match slice_worktree::existing_ref(&root, &branch_ref) {
        Ok(Some(slice_worktree::ExistingRef::Direct(actual)))
            if prepared_contract && actual == contract.base =>
        {
            failure["effects"]["branch"] = serde_json::json!({"state": "prepared"});
            true
        },
        Ok(Some(slice_worktree::ExistingRef::Direct(actual))) => {
            failure["effects"]["branch"] = serde_json::json!({"state": "conflicting", "detail": format!("branch points to {actual}")});
            false
        },
        Ok(Some(slice_worktree::ExistingRef::Symbolic(target))) => {
            failure["effects"]["branch"] = serde_json::json!({
                "state": "conflicting",
                "detail": format!("branch is symbolic to {target}")
            });
            false
        },
        Ok(None) => {
            failure["effects"]["branch"] = serde_json::json!({"state": "absent"});
            false
        },
        Err(detail) => {
            failure["effects"]["branch"] =
                serde_json::json!({"state": "unknown", "detail": detail});
            false
        },
    };
    let registered = slice_worktree::worktrees(&root).unwrap_or_default();
    let prepared_worktree = registered.iter().any(|worktree| {
        prepared_contract
            && branch_prepared
            && worktree.path == worktree_path
            && worktree.branch.as_deref() == Some(branch_ref.as_str())
            && worktree.head == contract.base
    });
    if prepared_worktree {
        failure["effects"]["worktree"] = serde_json::json!({"state": "prepared"});
        match slice_contract::binding_path_for(&worktree_path) {
            Ok(binding_path) => match fs::symlink_metadata(&binding_path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    failure["binding_path"] = serde_json::json!(binding_path);
                    failure["effects"]["binding"] = serde_json::json!({"state": "absent"});
                },
                Ok(_) => match slice_contract::verify_bound_exact(&worktree_path, &contract_path) {
                    Ok(binding_path) => {
                        failure["binding_path"] = serde_json::json!(binding_path);
                        failure["effects"]["binding"] = serde_json::json!({"state": "prepared"});
                    },
                    Err(detail) => {
                        failure["effects"]["binding"] =
                            serde_json::json!({"state": "conflicting", "detail": detail});
                    },
                },
                Err(error) => {
                    failure["effects"]["binding"] = serde_json::json!({
                        "state": "unknown",
                        "detail": format!("cannot inspect {}: {error}", binding_path.display())
                    });
                },
            },
            Err(detail) => {
                failure["effects"]["binding"] =
                    serde_json::json!({"state": "unknown", "detail": detail});
            },
        }
    } else if registered.iter().any(|worktree| {
        worktree.path == worktree_path || worktree.branch.as_deref() == Some(branch_ref.as_str())
    }) || worktree::path_entry_exists(&worktree_path).unwrap_or(false)
    {
        failure["effects"]["worktree"] = serde_json::json!({"state": "conflicting"});
    } else {
        failure["effects"]["worktree"] = serde_json::json!({"state": "absent"});
    }
    serde_json::to_string_pretty(&failure)
        .map_err(|error| format!("cannot encode Slice bootstrap failure: {error}"))
}
