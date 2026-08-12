use std::path::Path;

use super::{
    ExistingRef, existing_ref,
    model::{
        FailureRecord, Observation, ObservedEffects, ObservedState, RESULT_SCHEMA, Request,
        recover_contract_base,
    },
    storage,
};
use crate::{slice_contract, slice_worktree};

pub(super) fn failure(
    repository: &Path,
    request_bytes: Option<&[u8]>,
    initial_base: Option<String>,
    error: String,
) -> FailureRecord {
    let mut record = FailureRecord {
        schema: RESULT_SCHEMA,
        ok: false,
        slice: None,
        error,
        base: None,
        branch_ref: None,
        worktree_path: None,
        contract_path: None,
        binding_path: None,
        effects: unknown_effects("request was not validated".to_owned()),
    };
    let request = match request_bytes
        .ok_or_else(|| "request was not captured".to_owned())
        .and_then(read_request)
    {
        Ok(request) => request,
        Err(detail) => {
            record.effects = unknown_effects(detail);
            return record;
        },
    };
    record.slice = Some(request.slice.clone());
    let repository = match slice_worktree::repository_root(repository) {
        Ok(repository) => repository,
        Err(detail) => {
            record.effects = unknown_effects(detail);
            return record;
        },
    };
    let workspace = match slice_worktree::workspace_root(&repository) {
        Ok(workspace) => workspace,
        Err(detail) => {
            record.effects = unknown_effects(detail);
            return record;
        },
    };
    let contract_path = workspace
        .join(".local-exclude/coordination")
        .join(&request.slice)
        .join("slice-contract.json");
    let worktree_path = workspace
        .join(".local-exclude/worktrees")
        .join(&request.slice);
    let branch_ref = format!("refs/heads/slice/direct/{}", request.slice);
    record.contract_path = Some(contract_path.clone());
    record.worktree_path = Some(worktree_path.clone());
    record.branch_ref = Some(branch_ref.clone());

    let base = observe_contract(&mut record, &request, &contract_path, initial_base);
    record.base = base.clone();
    let contract_prepared = matches!(record.effects.contract.state, ObservedState::Prepared);
    record.effects.branch = match existing_ref(&repository, &branch_ref) {
        Ok(Some(ExistingRef::Direct(actual)))
            if contract_prepared && base.as_deref() == Some(actual.as_str()) =>
        {
            prepared()
        },
        Ok(Some(ExistingRef::Direct(actual))) => conflicting(format!("branch points to {actual}")),
        Ok(Some(ExistingRef::Symbolic(target))) => {
            conflicting(format!("branch is a symbolic ref to {target}"))
        },
        Ok(None) => absent(),
        Err(detail) => observation(ObservedState::Unknown, detail),
    };
    record.effects.worktree = observe_worktree(
        &repository,
        &worktree_path,
        &branch_ref,
        &base,
        contract_prepared,
    );
    observe_binding(&mut record, &worktree_path, &contract_path);
    record
}

fn read_request(bytes: &[u8]) -> Result<Request, String> {
    let request: Request = serde_json::from_slice(bytes)
        .map_err(|decode| format!("invalid activation Slice request: {decode}"))?;
    request.validate()?;
    Ok(request)
}

fn observe_contract(
    record: &mut FailureRecord,
    request: &Request,
    contract_path: &Path,
    initial_base: Option<String>,
) -> Option<String> {
    match storage::read_existing_contract(contract_path) {
        Ok(Some(bytes)) => match recover_contract_base(&bytes, request) {
            Ok(base) => {
                record.effects.contract = prepared();
                Some(base)
            },
            Err(detail) => {
                record.effects.contract = conflicting(detail);
                None
            },
        },
        Ok(None) => {
            record.effects.contract = absent();
            initial_base
        },
        Err(detail) => {
            record.effects.contract = conflicting(detail);
            None
        },
    }
}

fn observe_worktree(
    repository: &Path,
    expected_path: &Path,
    expected_branch: &str,
    expected_base: &Option<String>,
    contract_prepared: bool,
) -> Observation {
    match slice_worktree::worktrees(repository) {
        Ok(registered) => match registered.iter().find(|worktree| {
            worktree.path == expected_path || worktree.branch.as_deref() == Some(expected_branch)
        }) {
            Some(worktree)
                if contract_prepared
                    && worktree.path == expected_path
                    && worktree.branch.as_deref() == Some(expected_branch)
                    && expected_base.as_deref() == Some(worktree.head.as_str()) =>
            {
                prepared()
            },
            Some(_) => conflicting("registered worktree path, branch, or base differs"),
            None => match storage::path_entry_exists(expected_path) {
                Ok(true) => conflicting("unregistered worktree path exists"),
                Ok(false) => absent(),
                Err(detail) => observation(ObservedState::Unknown, detail),
            },
        },
        Err(detail) => observation(ObservedState::Unknown, detail),
    }
}

fn observe_binding(record: &mut FailureRecord, worktree_path: &Path, contract_path: &Path) {
    if !matches!(record.effects.worktree.state, ObservedState::Prepared) {
        record.effects.binding = observation(
            ObservedState::Unknown,
            "exact worktree is unavailable for binding inspection",
        );
        return;
    }
    match slice_contract::binding_path_for(worktree_path) {
        Ok(binding_path) => {
            record.binding_path = Some(binding_path.clone());
            record.effects.binding = match std::fs::symlink_metadata(&binding_path) {
                Ok(_) => match slice_contract::verify_bound_exact(worktree_path, contract_path) {
                    Ok(_) => prepared(),
                    Err(detail) => conflicting(detail),
                },
                Err(inspect) if inspect.kind() == std::io::ErrorKind::NotFound => absent(),
                Err(inspect) => observation(
                    ObservedState::Unknown,
                    format!("cannot inspect binding: {inspect}"),
                ),
            };
        },
        Err(detail) => {
            record.effects.binding = observation(ObservedState::Unknown, detail);
        },
    }
}

fn unknown_effects(detail: String) -> ObservedEffects {
    ObservedEffects {
        contract: observation(ObservedState::Unknown, detail.clone()),
        branch: observation(ObservedState::Unknown, detail.clone()),
        worktree: observation(ObservedState::Unknown, detail.clone()),
        binding: observation(ObservedState::Unknown, detail),
    }
}

fn prepared() -> Observation {
    Observation {
        state: ObservedState::Prepared,
        detail: None,
    }
}

fn absent() -> Observation {
    Observation {
        state: ObservedState::Absent,
        detail: None,
    }
}

fn conflicting(detail: impl Into<String>) -> Observation {
    observation(ObservedState::Conflicting, detail)
}

fn observation(state: ObservedState, detail: impl Into<String>) -> Observation {
    Observation {
        state,
        detail: Some(detail.into()),
    }
}
