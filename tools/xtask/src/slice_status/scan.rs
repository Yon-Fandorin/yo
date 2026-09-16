use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use super::{
    delivery,
    model::{
        Artifacts, CoordinationScope, EffectiveValidation, JSON_LIMIT, MAX_JSON_FILES,
        MAX_SCAN_DEPTH, ScanBudget, ValidationSummary,
    },
};
use crate::{bounded_file, review_protocol};

pub(super) fn scan_coordination(
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
    let mut gate_requests = Vec::new();
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
                    && evidence.hash == review_protocol::digest(&bytes)
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
        } else if schema == "yo.slice-gate-request/v1alpha1" {
            if value
                .get("candidate_commit")
                .and_then(serde_json::Value::as_str)
                == Some(scope.candidate)
            {
                found.gate_requests += 1;
                gate_requests.push(path.clone());
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
    if gate_requests.len() == 1 {
        found.gate_request = gate_requests.pop();
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
        if review_protocol::digest(&bytes) != hash {
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
    if review_protocol::digest(&egress_bytes) != egress_hash {
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
    if review_protocol::digest(&manifest_bytes) != manifest_hash {
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

pub(super) fn resolve_status_path(root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

pub(super) fn collect_json(
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
