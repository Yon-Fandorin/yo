use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{bounded_file, git, slice_contract, slice_worktree};

mod delivery;

const RESULT_SCHEMA: &str = "yo.slice-status/v1alpha3";
const JSON_LIMIT: usize = 8 * 1024 * 1024;
const MAX_JSON_FILES: usize = 256;
const MAX_SCAN_DEPTH: usize = 6;

#[derive(Clone, Debug)]
pub(crate) struct SliceState {
    pub(crate) worktree: PathBuf,
    pub(crate) branch: String,
    pub(crate) head: String,
    pub(crate) bound: slice_contract::BoundSlice,
    pub(crate) clean: bool,
}

struct Artifacts {
    validations: Vec<ValidationSummary>,
    gate_requests: usize,
    claims: usize,
    delivery_receipts: usize,
    review_rounds: usize,
    durable_requests: u64,
    prior_findings: usize,
    superseded: usize,
    delivery: delivery::Projection,
    delivery_request: Option<PathBuf>,
}

impl Default for Artifacts {
    fn default() -> Self {
        Self {
            validations: Vec::new(),
            gate_requests: 0,
            claims: 0,
            delivery_receipts: 0,
            review_rounds: 0,
            durable_requests: 0,
            prior_findings: 0,
            superseded: 0,
            delivery: delivery::prepared(),
            delivery_request: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct ValidationSummary {
    name: String,
    status: String,
    log_hash: String,
    path: String,
    reused: bool,
}

struct ReviewLineage {
    packets: usize,
    latest_candidate: Option<String>,
    status: &'static str,
    current_review_ids: BTreeSet<String>,
    latest_review_ids: BTreeSet<String>,
    current_validations: Vec<EffectiveValidation>,
}

struct EffectiveValidation {
    name: String,
    path: PathBuf,
    hash: String,
    reused: bool,
}

#[derive(Default)]
struct ScanBudget {
    json_files: usize,
}

struct CoordinationScope<'a> {
    repository: &'a Path,
    workspace: &'a Path,
    candidate: &'a str,
    current_review_ids: &'a BTreeSet<String>,
    latest_review_ids: &'a BTreeSet<String>,
    current_validations: &'a [EffectiveValidation],
}

#[derive(Serialize)]
struct ResultDocument {
    schema: &'static str,
    ok: bool,
    slice: String,
    branch: String,
    base_commit: String,
    candidate_commit: String,
    clean: bool,
    review_lineage: &'static str,
    review_packets: usize,
    review_rounds: usize,
    review_chain: Vec<String>,
    latest_packet_candidate: Option<String>,
    validation_summaries: Vec<ValidationSummary>,
    gate_requests: usize,
    delivery_claims: usize,
    delivery_receipts: usize,
    durable_external_requests: u64,
    superseded_artifacts: usize,
    delivery: delivery::Projection,
    #[serde(skip_serializing_if = "Option::is_none")]
    blocking_reason: Option<String>,
    next_action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_argv: Option<Vec<String>>,
}

pub(crate) fn run(repository: &Path, slice: &str) -> Result<(), String> {
    let state = locate(repository, slice)?;
    let workspace = slice_worktree::workspace_root(repository)?;
    let coordination = workspace
        .join(".local-exclude")
        .join("coordination")
        .join(slice);
    let mut budget = ScanBudget::default();
    let reviews = scan_review_lineage(&state, &workspace, &mut budget)?;
    let artifacts = scan_coordination(
        &coordination,
        &CoordinationScope {
            repository: &state.worktree,
            workspace: &workspace,
            candidate: &state.head,
            current_review_ids: &reviews.current_review_ids,
            latest_review_ids: &reviews.latest_review_ids,
            current_validations: &reviews.current_validations,
        },
        &mut budget,
    )?;
    let next_action = next_action(&state, &reviews, &artifacts);
    let (next_argv, blocking_reason) = next_invocation(slice, next_action, &artifacts);
    let result = ResultDocument {
        schema: RESULT_SCHEMA,
        ok: true,
        slice: slice.to_owned(),
        branch: state.branch,
        base_commit: state.bound.base,
        candidate_commit: state.head,
        clean: state.clean,
        review_lineage: reviews.status,
        review_packets: reviews.packets,
        review_rounds: artifacts.review_rounds,
        review_chain: reviews.current_review_ids.iter().cloned().collect(),
        latest_packet_candidate: reviews.latest_candidate,
        validation_summaries: artifacts.validations,
        gate_requests: artifacts.gate_requests,
        delivery_claims: artifacts.claims,
        delivery_receipts: artifacts.delivery_receipts,
        durable_external_requests: artifacts.durable_requests,
        superseded_artifacts: artifacts.superseded,
        delivery: artifacts.delivery,
        blocking_reason,
        next_action,
        next_argv,
    };
    println!(
        "{}",
        serde_json::to_string(&result)
            .map_err(|error| format!("cannot encode compact Slice status: {error}"))?
    );
    Ok(())
}

fn next_action(state: &SliceState, reviews: &ReviewLineage, artifacts: &Artifacts) -> &'static str {
    if !state.clean {
        "clean_candidate"
    } else if reviews.status == "broken" {
        "restore_review_lineage"
    } else if reviews.current_review_ids.is_empty() {
        if reviews
            .latest_candidate
            .as_deref()
            .is_some_and(|candidate| candidate != state.head)
            && artifacts.prior_findings > 0
        {
            "review_delta"
        } else {
            "build_review"
        }
    } else if artifacts.review_rounds == 0 {
        artifacts.delivery.next_action
    } else if artifacts.gate_requests == 0 {
        "prepare_gate"
    } else {
        "run_gate"
    }
}

fn next_invocation(
    slice: &str,
    action: &str,
    artifacts: &Artifacts,
) -> (Option<Vec<String>>, Option<String>) {
    match action {
        "deliver_current_review" => artifacts
            .delivery_request
            .as_ref()
            .map(|path| {
                (
                    Some(vec![
                        "cargo".to_owned(),
                        "xtask".to_owned(),
                        "slice".to_owned(),
                        "review-deliver".to_owned(),
                        path.display().to_string(),
                    ]),
                    None,
                )
            })
            .unwrap_or_else(|| {
                (
                    None,
                    Some("no current immutable delivery request is published".to_owned()),
                )
            }),
        "await_current_delivery" => (
            Some(vec![
                "cargo".to_owned(),
                "xtask".to_owned(),
                "slice".to_owned(),
                "status".to_owned(),
                slice.to_owned(),
            ]),
            artifacts.delivery.blocking_reason.clone(),
        ),
        "interpret_review" | "reconcile_failed_delivery" | "reconcile_unknown_delivery" => {
            (None, artifacts.delivery.blocking_reason.clone())
        },
        _ => (
            None,
            Some(format!(
                "`{action}` requires an explicit content-addressed request before an exact argv exists"
            )),
        ),
    }
}

pub(crate) fn locate(repository: &Path, slice: &str) -> Result<SliceState, String> {
    validate_slice_name(slice)?;
    let mut matches = Vec::new();
    for worktree in slice_worktree::worktrees(repository)? {
        let Some(branch) = worktree.branch.as_deref() else {
            continue;
        };
        if !branch_names_slice(branch, slice) {
            continue;
        }
        let bound = slice_contract::bound_slice(&worktree.path)?;
        if bound.slice == slice {
            matches.push((worktree, bound));
        }
    }
    let (worktree, bound) = match matches.as_slice() {
        [(worktree, bound)] => (worktree, bound),
        [] => return Err(format!("no registered Slice worktree found for `{slice}`")),
        _ => {
            return Err(format!(
                "multiple registered Slice worktrees match `{slice}`"
            ));
        },
    };
    let clean = git::output_bytes_in(
        &worktree.path,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        false,
    )?
    .is_empty();
    Ok(SliceState {
        worktree: worktree.path.clone(),
        branch: worktree
            .branch
            .clone()
            .expect("matched Slice worktree has a branch"),
        head: worktree.head.clone(),
        bound: bound.clone(),
        clean,
    })
}

fn branch_names_slice(branch: &str, slice: &str) -> bool {
    branch == format!("refs/heads/slice/direct/{slice}")
        || branch
            .strip_prefix("refs/heads/slice/")
            .and_then(|rest| rest.split_once('/'))
            .is_some_and(|(wave, name)| !wave.is_empty() && name == slice)
}

fn validate_slice_name(slice: &str) -> Result<(), String> {
    if slice.is_empty()
        || slice != slice.trim()
        || slice.contains('/')
        || matches!(slice, "." | "..")
    {
        Err("Slice name must be one non-empty branch segment".to_owned())
    } else {
        Ok(())
    }
}

fn scan_coordination(
    root: &Path,
    scope: &CoordinationScope<'_>,
    budget: &mut ScanBudget,
) -> Result<Artifacts, String> {
    if !root.exists() {
        return Ok(Artifacts::default());
    }
    let mut files = Vec::new();
    collect_json(root, 0, &mut files, budget)?;
    let mut found = Artifacts::default();
    let mut values = Vec::new();
    let mut current_claims = Vec::new();
    let mut delivery_requests = Vec::new();
    for path in files {
        let bytes = bounded_file::read_regular(&path, JSON_LIMIT, "Slice coordination JSON")?;
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let Some(schema) = value.get("schema").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if schema.starts_with("yo.validation-run-summary/") {
            let name = value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let effective = scope.current_validations.iter().find(|evidence| {
                evidence.name == name
                    && evidence.path == path
                    && evidence.hash == crate::review_protocol::digest(&bytes)
            });
            let current = if scope.current_review_ids.is_empty() {
                value.get("head_commit").and_then(serde_json::Value::as_str)
                    == Some(scope.candidate)
            } else {
                effective.is_some()
            };
            if current {
                found.validations.push(ValidationSummary {
                    name: name.to_owned(),
                    status: value
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown")
                        .to_owned(),
                    log_hash: value
                        .get("log_hash")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown")
                        .to_owned(),
                    path: path.display().to_string(),
                    reused: effective.is_some_and(|evidence| evidence.reused),
                });
            } else {
                found.superseded += 1;
            }
        } else if schema.starts_with("yo.slice-gate-request/") {
            if value
                .get("candidate_commit")
                .and_then(serde_json::Value::as_str)
                == Some(scope.candidate)
            {
                found.gate_requests += 1;
            } else {
                found.superseded += 1;
            }
        } else if schema == "yo.slice-review-findings/v1"
            && value
                .get("review_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|review_id| scope.latest_review_ids.contains(review_id))
        {
            found.prior_findings += 1;
        } else if schema.contains("delivery-claim/")
            && value
                .get("candidate_commit")
                .and_then(serde_json::Value::as_str)
                == Some(scope.candidate)
            && value
                .get("review_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|review_id| scope.current_review_ids.contains(review_id))
        {
            found.claims += 1;
            if let Some(request_id) = value.get("request_id").and_then(serde_json::Value::as_str) {
                current_claims.push((
                    request_id.to_owned(),
                    path.parent().unwrap_or(root).to_path_buf(),
                ));
            }
        } else if schema.contains("delivery-claim/") {
            found.superseded += 1;
        } else if is_delivery_request_schema(schema) {
            match delivery_request_review_id(scope.repository, scope.workspace, &value)? {
                Some(review_id) if scope.current_review_ids.contains(&review_id) => {
                    delivery_requests.push(path.clone());
                },
                Some(_) => found.superseded += 1,
                None => {},
            }
        }
        values.push((path, value));
    }
    let mut completed_reviews = BTreeSet::new();
    let mut outcomes = BTreeMap::<String, (String, u64)>::new();
    for (_, value) in &values {
        let schema = value
            .get("schema")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if schema.contains("delivery-receipt/")
            && let Some(review_id) = value
                .get("review_id")
                .and_then(serde_json::Value::as_str)
                .filter(|review_id| scope.current_review_ids.contains(*review_id))
        {
            found.delivery_receipts += 1;
            completed_reviews.insert(review_id.to_owned());
        }
        if schema.contains("delivery-outcome/")
            && let Some(request_id) = value.get("request_id").and_then(serde_json::Value::as_str)
        {
            let durable = value
                .get("durable_host_request_count")
                .or_else(|| value.get("durable_provider_request_count"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            outcomes.insert(
                request_id.to_owned(),
                (
                    value
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown")
                        .to_owned(),
                    durable,
                ),
            );
        }
    }
    found.review_rounds = completed_reviews.len();
    let has_current_receipt = !completed_reviews.is_empty();
    let attempts = current_claims
        .into_iter()
        .map(|(request_id, output_directory)| {
            let outcome = outcomes.get(&request_id);
            delivery::AttemptInput {
                request_id,
                output_directory,
                outcome_status: outcome.map(|(status, _)| status.clone()),
                outcome_durable_requests: outcome.map(|(_, durable)| *durable),
                has_receipt: has_current_receipt,
            }
        })
        .collect::<Vec<_>>();
    found.delivery = delivery::project(attempts)?;
    found.durable_requests = found.delivery.durable_request_count;
    found.validations.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.path.cmp(&right.path))
    });
    if delivery_requests.len() == 1 {
        found.delivery_request = delivery_requests.pop();
    }
    Ok(found)
}

fn is_delivery_request_schema(schema: &str) -> bool {
    matches!(
        schema.split('/').next(),
        Some(
            "yo.slice-review-delivery-request"
                | "yo.slice-review-delegated-delivery-request"
                | "yo.slice-review-continuation-delivery-request"
                | "yo.slice-review-delegated-continuation-delivery-request"
        )
    )
}

fn delivery_request_review_id(
    repository: &Path,
    workspace: &Path,
    request: &serde_json::Value,
) -> Result<Option<String>, String> {
    let (egress_path, egress_hash, egress_is_shared) = if let (Some(path), Some(hash)) = (
        request
            .get("egress_request_path")
            .and_then(serde_json::Value::as_str),
        request
            .get("egress_request_hash")
            .and_then(serde_json::Value::as_str),
    ) {
        (path.to_owned(), hash.to_owned(), true)
    } else if let (Some(path), Some(hash)) = (
        request
            .get("preflight_request_path")
            .and_then(serde_json::Value::as_str),
        request
            .get("preflight_request_hash")
            .and_then(serde_json::Value::as_str),
    ) {
        let path = resolve_status_path(workspace, path);
        let bytes = bounded_file::read_regular(&path, JSON_LIMIT, "review continuation preflight")?;
        if crate::review_protocol::digest(&bytes) != hash {
            return Err("review continuation preflight hash changed".to_owned());
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid review continuation preflight: {error}"))?;
        let Some(egress_path) = value
            .get("egress_request_path")
            .and_then(serde_json::Value::as_str)
        else {
            return Ok(None);
        };
        let Some(egress_hash) = value
            .get("egress_request_hash")
            .and_then(serde_json::Value::as_str)
        else {
            return Ok(None);
        };
        (egress_path.to_owned(), egress_hash.to_owned(), false)
    } else {
        return Ok(None);
    };
    let egress_path = resolve_status_path(
        if egress_is_shared {
            workspace
        } else {
            repository
        },
        &egress_path,
    );
    let egress_bytes =
        bounded_file::read_regular(&egress_path, JSON_LIMIT, "review egress request")?;
    if crate::review_protocol::digest(&egress_bytes) != egress_hash {
        return Err("review egress request hash changed".to_owned());
    }
    let egress: serde_json::Value = serde_json::from_slice(&egress_bytes)
        .map_err(|error| format!("invalid review egress request: {error}"))?;
    let Some(manifest_path) = egress
        .get("manifest_path")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(None);
    };
    let Some(manifest_hash) = egress
        .get("manifest_hash")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(None);
    };
    let manifest_path = resolve_status_path(repository, manifest_path);
    let manifest_bytes =
        bounded_file::read_regular(&manifest_path, JSON_LIMIT, "review-chain manifest")?;
    if crate::review_protocol::digest(&manifest_bytes) != manifest_hash {
        return Err("review-chain manifest hash changed".to_owned());
    }
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid review-chain manifest: {error}"))?;
    Ok(manifest
        .get("review_id")
        .or_else(|| manifest.get("review_delta_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned))
}

fn resolve_status_path(root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn scan_review_lineage(
    state: &SliceState,
    workspace: &Path,
    budget: &mut ScanBudget,
) -> Result<ReviewLineage, String> {
    let mut files = Vec::new();
    for root in [
        state.worktree.join(".local-exclude/methexis"),
        workspace.join(".local-exclude/methexis"),
    ] {
        if root.exists() {
            collect_json(&root, 0, &mut files, budget)?;
        }
    }
    files.sort();
    files.dedup();
    let mut candidates = BTreeSet::new();
    let mut review_ids = BTreeMap::<String, BTreeSet<String>>::new();
    let mut current_review_ids = BTreeSet::new();
    let mut current_validations = BTreeMap::<String, EffectiveValidation>::new();
    let mut broken = false;
    for path in files {
        if path.file_name().and_then(|name| name.to_str()) != Some("manifest.json") {
            continue;
        }
        let bytes = bounded_file::read_regular(&path, JSON_LIMIT, "published review manifest")?;
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        if value
            .pointer("/plan/slice_contract/hash")
            .and_then(serde_json::Value::as_str)
            != Some(state.bound.contract_id.as_str())
        {
            continue;
        }
        let candidate = value
            .pointer("/plan/candidate_commit")
            .or_else(|| value.pointer("/plan/replacement_candidate_commit"))
            .and_then(serde_json::Value::as_str);
        let Some(candidate) = candidate else { continue };
        if !is_commit(candidate) {
            broken = true;
            continue;
        }
        candidates.insert(candidate.to_owned());
        if let Some(review_id) = value
            .get("review_id")
            .or_else(|| value.get("review_delta_id"))
            .and_then(serde_json::Value::as_str)
        {
            review_ids
                .entry(candidate.to_owned())
                .or_default()
                .insert(review_id.to_owned());
            if candidate == state.head {
                current_review_ids.insert(review_id.to_owned());
                for validation in manifest_validations(&value, &state.worktree)? {
                    if current_validations
                        .insert(validation.name.clone(), validation)
                        .is_some()
                    {
                        broken = true;
                    }
                }
            }
        }
        if !git::trusted_succeeds_in(
            &state.worktree,
            &["merge-base", "--is-ancestor", candidate, &state.head],
        )? {
            broken = true;
        }
    }
    let mut latest = None;
    let mut smallest_distance = u64::MAX;
    for candidate in &candidates {
        if !git::trusted_succeeds_in(
            &state.worktree,
            &["merge-base", "--is-ancestor", candidate, &state.head],
        )? {
            continue;
        }
        let count = git::trusted_output_in(
            &state.worktree,
            &[
                "rev-list",
                "--count",
                &format!("{candidate}..{}", state.head),
            ],
        )?;
        let distance = count
            .trim()
            .parse::<u64>()
            .map_err(|error| format!("Git returned invalid review distance: {error}"))?;
        if distance < smallest_distance {
            smallest_distance = distance;
            latest = Some(candidate.clone());
        }
    }
    let latest_review_ids = latest
        .as_ref()
        .and_then(|candidate| review_ids.get(candidate))
        .cloned()
        .unwrap_or_default();
    Ok(ReviewLineage {
        packets: candidates.len(),
        latest_candidate: latest,
        status: if broken { "broken" } else { "preserved" },
        current_review_ids,
        latest_review_ids,
        current_validations: current_validations.into_values().collect(),
    })
}

fn manifest_validations(
    manifest: &serde_json::Value,
    repository: &Path,
) -> Result<Vec<EffectiveValidation>, String> {
    let mut result = Vec::new();
    for (pointer, reused) in [
        ("/inputs/validation_evidence", false),
        ("/inputs/reused_validation_evidence", true),
        ("/inputs/affected_validation_evidence", false),
    ] {
        let Some(entries) = manifest
            .pointer(pointer)
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for entry in entries {
            let Some(name) = entry.get("name").and_then(serde_json::Value::as_str) else {
                return Err("review manifest validation evidence has no name".to_owned());
            };
            let Some(path) = entry
                .pointer("/artifact/path")
                .and_then(serde_json::Value::as_str)
            else {
                return Err("review manifest validation evidence has no artifact path".to_owned());
            };
            let Some(hash) = entry
                .pointer("/artifact/hash")
                .and_then(serde_json::Value::as_str)
            else {
                return Err("review manifest validation evidence has no artifact hash".to_owned());
            };
            result.push(EffectiveValidation {
                name: name.to_owned(),
                path: resolve_status_path(repository, path),
                hash: hash.to_owned(),
                reused,
            });
        }
    }
    Ok(result)
}

fn collect_json(
    directory: &Path,
    depth: usize,
    output: &mut Vec<PathBuf>,
    budget: &mut ScanBudget,
) -> Result<(), String> {
    if depth > MAX_SCAN_DEPTH {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        format!(
            "cannot inspect status path {}: {error}",
            directory.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(());
    }
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot scan status path {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot read status path {}: {error}", directory.display()))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect status entry {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_json(&path, depth + 1, output, budget)?;
        } else if metadata.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("json")
        {
            if budget.json_files == MAX_JSON_FILES {
                return Err(format!(
                    "compact Slice status exceeded its global {MAX_JSON_FILES}-JSON-file scan limit"
                ));
            }
            budget.json_files += 1;
            output.push(path);
        }
    }
    Ok(())
}

fn is_commit(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestRepository;

    fn state(head: &str) -> SliceState {
        SliceState {
            worktree: PathBuf::from("/tmp/example"),
            branch: "refs/heads/slice/direct/example".to_owned(),
            head: head.to_owned(),
            bound: slice_contract::BoundSlice {
                slice: "example".to_owned(),
                base: "base".to_owned(),
                base_ref: "refs/heads/develop".to_owned(),
                binding_path: PathBuf::from("binding"),
                contract_path: PathBuf::from("contract"),
                contract_id: "sha256:contract".to_owned(),
            },
            clean: true,
        }
    }

    // 현재 후보 packet이 이미 있으면 같은 packet을 다시 만들라고 하지 않고 delivery로,
    // 이전 후보만 있으면 전체 packet 대신 review delta로 다음 행동을 구분합니다.
    #[test]
    fn next_action_reuses_current_review_or_selects_delta() {
        let current = ReviewLineage {
            packets: 1,
            latest_candidate: Some("current".to_owned()),
            status: "preserved",
            current_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
            latest_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
            current_validations: Vec::new(),
        };
        assert_eq!(
            next_action(&state("current"), &current, &Artifacts::default()),
            "deliver_current_review"
        );

        let prior = ReviewLineage {
            packets: 1,
            latest_candidate: Some("prior".to_owned()),
            status: "preserved",
            current_review_ids: BTreeSet::new(),
            latest_review_ids: BTreeSet::from(["sha256:prior".to_owned()]),
            current_validations: Vec::new(),
        };
        assert_eq!(
            next_action(
                &state("current"),
                &prior,
                &Artifacts {
                    prior_findings: 1,
                    ..Artifacts::default()
                },
            ),
            "review_delta"
        );
        assert_eq!(
            next_action(&state("current"), &prior, &Artifacts::default()),
            "build_review"
        );
    }

    // receipt와 current-candidate gate가 있으면 review/gate 생성을 반복하지 않고 기존
    // exact gate 실행으로 바로 이어져 content-addressed reuse 경계를 보존합니다.
    #[test]
    fn next_action_reuses_existing_gate_inputs() {
        let reviews = ReviewLineage {
            packets: 1,
            latest_candidate: Some("current".to_owned()),
            status: "preserved",
            current_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
            latest_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
            current_validations: Vec::new(),
        };
        let artifacts = Artifacts {
            review_rounds: 1,
            gate_requests: 1,
            ..Artifacts::default()
        };

        assert_eq!(
            next_action(&state("current"), &reviews, &artifacts),
            "run_gate"
        );
    }

    // Slice 이름과 branch matcher는 임의 경로나 다른 Wave의 접두사를 느슨하게
    // 받아들이지 않고 direct 또는 정확한 한 Wave branch만 선택합니다.
    #[test]
    fn branch_matching_is_exact() {
        assert!(branch_names_slice(
            "refs/heads/slice/direct/example",
            "example"
        ));
        assert!(branch_names_slice(
            "refs/heads/slice/wave-a/example",
            "example"
        ));
        assert!(!branch_names_slice(
            "refs/heads/slice/wave-a/example/extra",
            "example"
        ));
        assert!(validate_slice_name("../example").is_err());
    }

    // 이미 발행한 후보를 reset으로 버리고 다른 child를 만들면 manifest 자체가 남아
    // 있어도 ancestry guard가 rewritten history를 preserved로 오인하지 않습니다.
    #[test]
    fn review_lineage_detects_replaced_candidate() {
        let repository = TestRepository::new("slice-status-lineage");
        repository.write("tracked.txt", "base\n");
        repository.git(["add", "tracked.txt"]);
        repository.git(["commit", "--quiet", "-m", "base"]);
        let base = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
            .unwrap()
            .trim()
            .to_owned();
        repository.write("tracked.txt", "reviewed\n");
        repository.git(["add", "tracked.txt"]);
        repository.git(["commit", "--quiet", "-m", "reviewed"]);
        let reviewed = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
            .unwrap()
            .trim()
            .to_owned();
        repository.git(["reset", "--hard", &base]);
        repository.write("tracked.txt", "replacement\n");
        repository.git(["add", "tracked.txt"]);
        repository.git(["commit", "--quiet", "-m", "replacement"]);
        let replacement = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
            .unwrap()
            .trim()
            .to_owned();
        repository.write(
            ".local-exclude/methexis/review/manifest.json",
            &format!(
                "{{\"plan\":{{\"candidate_commit\":\"{reviewed}\",\"slice_contract\":{{\"hash\":\"sha256:contract\"}}}}}}\n"
            ),
        );
        let state = SliceState {
            worktree: repository.path.clone(),
            branch: "refs/heads/slice/direct/example".to_owned(),
            head: replacement,
            bound: slice_contract::BoundSlice {
                slice: "example".to_owned(),
                base,
                base_ref: "refs/heads/develop".to_owned(),
                binding_path: repository.path.join("binding"),
                contract_path: repository.path.join("contract"),
                contract_id: "sha256:contract".to_owned(),
            },
            clean: true,
        };

        let reviews =
            scan_review_lineage(&state, &repository.path, &mut ScanBudget::default()).unwrap();
        assert_eq!(reviews.packets, 1);
        assert_eq!(reviews.latest_candidate, None);
        assert_eq!(reviews.status, "broken");
    }

    // 이전 후보의 validation/gate/claim 파일이 coordination에 남아 있어도 현재 HEAD의
    // 완료 수로 합산하지 않아 compact status가 stale progress를 만들지 않습니다.
    #[test]
    fn coordination_counts_only_candidate_bound_progress() {
        let root = crate::test_support::unique_path("slice-status-coordination");
        fs::create_dir_all(&root).unwrap();
        for (name, value) in [
            (
                "current-validation.json",
                serde_json::json!({
                    "schema": "yo.validation-run-summary/v1alpha2",
                    "name": "current-validation",
                    "status": "passed",
                    "log_hash": "sha256:log",
                    "head_commit": "current"
                }),
            ),
            (
                "stale-validation.json",
                serde_json::json!({
                    "schema": "yo.validation-run-summary/v1alpha2",
                    "head_commit": "stale"
                }),
            ),
            (
                "stale-gate.json",
                serde_json::json!({
                    "schema": "yo.slice-gate-request/v1alpha1",
                    "candidate_commit": "stale"
                }),
            ),
            (
                "current-receipt.json",
                serde_json::json!({
                    "schema": "yo.external-review-delivery-receipt/v1",
                    "review_id": "sha256:current-review"
                }),
            ),
            (
                "prior-findings.json",
                serde_json::json!({
                    "schema": "yo.slice-review-findings/v1",
                    "review_id": "sha256:current-review",
                    "candidate_commit": "reviewed"
                }),
            ),
            (
                "stale-receipt.json",
                serde_json::json!({
                    "schema": "yo.external-review-delivery-receipt/v1",
                    "review_id": "sha256:stale-review"
                }),
            ),
        ] {
            fs::write(root.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
        }

        let current_validation_path = root.join("current-validation.json");
        let current_validation_hash =
            crate::review_protocol::digest(&fs::read(&current_validation_path).unwrap());
        let effective = [EffectiveValidation {
            name: "current-validation".to_owned(),
            path: current_validation_path,
            hash: current_validation_hash,
            reused: true,
        }];
        let artifacts = scan_coordination(
            &root,
            &CoordinationScope {
                repository: &root,
                workspace: &root,
                candidate: "current",
                current_review_ids: &BTreeSet::from(["sha256:current-review".to_owned()]),
                latest_review_ids: &BTreeSet::from(["sha256:current-review".to_owned()]),
                current_validations: &effective,
            },
            &mut ScanBudget::default(),
        )
        .unwrap();
        assert_eq!(artifacts.validations.len(), 1);
        assert!(artifacts.validations[0].reused);
        assert_eq!(artifacts.gate_requests, 0);
        assert_eq!(artifacts.delivery_receipts, 1);
        assert_eq!(artifacts.review_rounds, 1);
        assert_eq!(artifacts.prior_findings, 1);
        fs::remove_dir_all(root).unwrap();
    }

    // claim은 외부 효과가 아직 관측되지 않았더라도 exact-once 소유권을 소비하므로 compact
    // coordinator는 같은 review에 대한 두 번째 delivery 명령을 절대 제안하지 않습니다.
    #[test]
    fn current_claim_blocks_a_second_delivery() {
        let root = crate::test_support::unique_path("slice-status-current-claim");
        fs::create_dir_all(root.join("attempt")).unwrap();
        fs::write(
            root.join("attempt/claim.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema": "yo.external-review-delivery-claim/v1alpha2",
                "request_id": "sha256:request",
                "review_id": "sha256:review",
                "candidate_commit": "current"
            }))
            .unwrap(),
        )
        .unwrap();

        let artifacts = scan_coordination(
            &root,
            &CoordinationScope {
                repository: &root,
                workspace: &root,
                candidate: "current",
                current_review_ids: &BTreeSet::from(["sha256:review".to_owned()]),
                latest_review_ids: &BTreeSet::from(["sha256:review".to_owned()]),
                current_validations: &[],
            },
            &mut ScanBudget::default(),
        )
        .unwrap();
        let reviews = ReviewLineage {
            packets: 1,
            latest_candidate: Some("current".to_owned()),
            status: "preserved",
            current_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
            latest_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
            current_validations: Vec::new(),
        };
        assert_eq!(artifacts.delivery.state, delivery::State::Claimed);
        assert_eq!(
            next_action(&state("current"), &reviews, &artifacts),
            "await_current_delivery"
        );
        fs::remove_dir_all(root).unwrap();
    }

    // coordination에 이전 후보의 유일한 delivery request만 남아 있어도 그 request가
    // 가리키는 manifest ReviewId를 다시 결속해 현재 review의 exact argv로 제안하지 않습니다.
    #[test]
    fn stale_delivery_request_is_not_current_next_argv() {
        let repository = crate::test_support::unique_path("slice-status-stale-delivery");
        let coordination = repository.join("coordination");
        let manifest_path = repository.join("stale-manifest.json");
        fs::create_dir_all(&coordination).unwrap();
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schema": "yo.slice-review-manifest/v1",
            "review_id": "sha256:stale-review"
        }))
        .unwrap();
        fs::write(&manifest_path, &manifest).unwrap();
        let egress = serde_json::to_vec(&serde_json::json!({
            "schema": "yo.slice-review-delegated-egress-request/v1alpha1",
            "manifest_path": manifest_path.display().to_string(),
            "manifest_hash": crate::review_protocol::digest(&manifest)
        }))
        .unwrap();
        let egress_path = coordination.join("stale-egress.json");
        fs::write(&egress_path, &egress).unwrap();
        fs::write(
            coordination.join("review-delivery.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema": "yo.slice-review-delegated-delivery-request/v1alpha2",
                "egress_request_path": egress_path.display().to_string(),
                "egress_request_hash": crate::review_protocol::digest(&egress)
            }))
            .unwrap(),
        )
        .unwrap();

        let artifacts = scan_coordination(
            &coordination,
            &CoordinationScope {
                repository: &repository,
                workspace: &repository,
                candidate: "current",
                current_review_ids: &BTreeSet::from(["sha256:current-review".to_owned()]),
                latest_review_ids: &BTreeSet::from(["sha256:current-review".to_owned()]),
                current_validations: &[],
            },
            &mut ScanBudget::default(),
        )
        .unwrap();
        assert!(artifacts.delivery_request.is_none());
        assert_eq!(artifacts.superseded, 1);
        fs::remove_dir_all(repository).unwrap();
    }

    // original manifest의 review_id와 finding-resolution manifest의 review_delta_id는
    // 서로 다른 wire 필드이므로 현재 replacement 후보의 receipt 연결은 둘 다 읽습니다.
    #[test]
    fn review_lineage_identifies_current_delta_review() {
        let repository = TestRepository::new("slice-status-delta-review");
        repository.write("tracked.txt", "base\n");
        repository.git(["add", "tracked.txt"]);
        repository.git(["commit", "--quiet", "-m", "base"]);
        let base = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
            .unwrap()
            .trim()
            .to_owned();
        repository.write("tracked.txt", "replacement\n");
        repository.git(["add", "tracked.txt"]);
        repository.git(["commit", "--quiet", "-m", "replacement"]);
        let replacement = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
            .unwrap()
            .trim()
            .to_owned();
        repository.write(
            ".local-exclude/methexis/delta/manifest.json",
            &format!(
                "{{\"review_delta_id\":\"sha256:delta\",\"plan\":{{\"replacement_candidate_commit\":\"{replacement}\",\"slice_contract\":{{\"hash\":\"sha256:contract\"}}}}}}\n"
            ),
        );
        let state = SliceState {
            worktree: repository.path.clone(),
            branch: "refs/heads/slice/direct/example".to_owned(),
            head: replacement,
            bound: slice_contract::BoundSlice {
                slice: "example".to_owned(),
                base,
                base_ref: "refs/heads/develop".to_owned(),
                binding_path: repository.path.join("binding"),
                contract_path: repository.path.join("contract"),
                contract_id: "sha256:contract".to_owned(),
            },
            clean: true,
        };

        let reviews =
            scan_review_lineage(&state, &repository.path, &mut ScanBudget::default()).unwrap();
        assert_eq!(
            reviews.current_review_ids,
            BTreeSet::from(["sha256:delta".to_owned()])
        );
    }

    // manifest와 coordination은 하나의 전역 예산을 공유해 두 scan이 각각 256개를
    // 읽는 방식으로 문서화된 bounded 입력 한도를 우회하지 않습니다.
    #[test]
    fn json_scan_budget_is_global_across_roots() {
        let first = crate::test_support::unique_path("slice-status-budget-first");
        let second = crate::test_support::unique_path("slice-status-budget-second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        for index in 0..128 {
            fs::write(first.join(format!("{index:03}.json")), b"{}\n").unwrap();
            fs::write(second.join(format!("{index:03}.json")), b"{}\n").unwrap();
        }
        let mut budget = ScanBudget::default();
        let mut files = Vec::new();
        collect_json(&first, 0, &mut files, &mut budget).unwrap();
        collect_json(&second, 0, &mut files, &mut budget).unwrap();
        fs::write(second.join("overflow.json"), b"{}\n").unwrap();
        assert!(
            collect_json(&second, 0, &mut Vec::new(), &mut budget)
                .unwrap_err()
                .contains("global 256-JSON-file")
        );
        fs::remove_dir_all(first).unwrap();
        fs::remove_dir_all(second).unwrap();
    }
}
