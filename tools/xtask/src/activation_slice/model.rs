use std::{collections::BTreeSet, path::PathBuf};

use serde::{Deserialize, Serialize};

pub(super) const REQUEST_SCHEMA: &str = "yo.activation-slice-request/v1";
pub(super) const RESULT_SCHEMA: &str = "yo.activation-slice-bootstrap/v1";
pub(super) const CONTRACT_SCHEMA: &str = "yo.slice-contract/v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) schema: String,
    pub(super) slice: String,
    pub(super) owned_contracts: Vec<String>,
    #[serde(default)]
    pub(super) dependencies: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct Contract<'a> {
    pub(super) schema: &'static str,
    pub(super) slice: &'a str,
    pub(super) base: &'a str,
    pub(super) base_ref: &'static str,
    pub(super) owned_contracts: &'a [String],
    pub(super) dependencies: &'a [String],
    pub(super) allowed_write_set: [&'static str; 4],
    pub(super) focused_checks: [&'static str; 1],
    pub(super) slice_close_checks: [&'static str; 1],
}

#[derive(Deserialize)]
struct StoredContract {
    base: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ResultRecord {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) slice: String,
    pub(super) base: String,
    pub(super) branch_ref: String,
    pub(super) worktree_path: PathBuf,
    pub(super) contract_path: PathBuf,
    pub(super) binding_path: PathBuf,
    pub(super) effects: Effects,
}

#[derive(Debug, Serialize)]
pub(super) struct Effects {
    pub(super) contract: Effect,
    pub(super) branch: Effect,
    pub(super) worktree: Effect,
    pub(super) binding: Effect,
}

#[derive(Debug, Serialize)]
pub(super) struct FailureRecord {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) slice: Option<String>,
    pub(super) error: String,
    pub(super) base: Option<String>,
    pub(super) branch_ref: Option<String>,
    pub(super) worktree_path: Option<PathBuf>,
    pub(super) contract_path: Option<PathBuf>,
    pub(super) binding_path: Option<PathBuf>,
    pub(super) effects: ObservedEffects,
}

#[derive(Debug, Serialize)]
pub(super) struct ObservedEffects {
    pub(super) contract: Observation,
    pub(super) branch: Observation,
    pub(super) worktree: Observation,
    pub(super) binding: Observation,
}

#[derive(Debug, Serialize)]
pub(super) struct Observation {
    pub(super) state: ObservedState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) detail: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ObservedState {
    Prepared,
    Absent,
    Conflicting,
    Unknown,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Effect {
    Created,
    Reused,
}

impl Request {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.schema != REQUEST_SCHEMA {
            return Err(format!(
                "unsupported activation Slice request schema `{}`; expected `{REQUEST_SCHEMA}`",
                self.schema
            ));
        }
        validate_slice_name(&self.slice)?;
        if self.owned_contracts.is_empty() {
            return Err("activation Slice request must declare an owned contract".to_owned());
        }
        validate_values("owned contract", &self.owned_contracts)?;
        validate_values("dependency", &self.dependencies)
    }
}

fn validate_slice_name(slice: &str) -> Result<(), String> {
    if slice.is_empty()
        || slice != slice.trim()
        || slice.contains('/')
        || matches!(slice, "." | "..")
    {
        return Err("Slice name must be one non-empty branch segment".to_owned());
    }
    Ok(())
}

fn validate_values(label: &str, values: &[String]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.is_empty() || value != value.trim() {
            return Err(format!(
                "{label} must be non-empty and have no surrounding whitespace"
            ));
        }
        if !seen.insert(value) {
            return Err(format!("duplicate {label} `{value}`"));
        }
    }
    Ok(())
}

pub(super) fn contract_bytes<'a>(request: &'a Request, base: &'a str) -> Result<Vec<u8>, String> {
    let contract = Contract {
        schema: CONTRACT_SCHEMA,
        slice: &request.slice,
        base,
        base_ref: "refs/heads/develop",
        owned_contracts: &request.owned_contracts,
        dependencies: &request.dependencies,
        allowed_write_set: [
            "methexis/active-checkpoint.yaml",
            "methexis/checkpoints/**",
            "tools/methexis/examples/context-contract/manifest.json",
            "tools/methexis/examples/context-contract/stable-leaf-manifest.json",
        ],
        focused_checks: ["cargo run --locked -p methexis -- check --staged-activation"],
        slice_close_checks: ["git diff --check"],
    };
    let mut bytes = serde_json::to_vec_pretty(&contract)
        .map_err(|error| format!("cannot encode activation Slice contract: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(super) fn recover_contract_base(bytes: &[u8], request: &Request) -> Result<String, String> {
    let stored: StoredContract = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid existing activation Slice contract: {error}"))?;
    let expected = contract_bytes(request, &stored.base)?;
    if expected == bytes {
        Ok(stored.base)
    } else {
        Err("existing activation Slice contract does not match the exact request".to_owned())
    }
}
