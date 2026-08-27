mod model;

#[cfg(test)]
mod tests;

use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

use rustix::fs::Dir;
use serde::Serialize;

use self::model::{
    CONTEXT_SCHEMA, ContextAnchor, ContextRequest, DELEGATED_ADMISSION_SCHEMA,
    DELEGATED_DELIVERY_SCHEMA, DELEGATED_EGRESS_SCHEMA, DELEGATED_EXECUTION_PROFILE,
    DELIVERY_PROFILE, DelegatedAdmission, DelegatedEgress, DelegatedTarget, DeliveryRequest,
    FreshSession, MANAGED_ADMISSION_SCHEMA, MANAGED_DELIVERY_SCHEMA, MANAGED_EGRESS_SCHEMA,
    ManagedAdmission, ManagedAdmissionTarget, ManagedEgress, ManagedRoute, REQUEST_SCHEMA,
    RESULT_SCHEMA, REVIEW_SCHEMA, Request, ReviewRequest, TOKENIZER_PROFILE, Target,
};
use crate::{
    bounded_file, review_egress, review_packet,
    review_protocol::{digest, relative},
    review_target_admission, slice_contract, slice_worktree,
};

const REQUEST_LIMIT: usize = 256 * 1024;
const GENERATED_REQUEST_LIMIT: usize = 256 * 1024;
const AUTHORIZATION_LIMIT: usize = 64 * 1024;
const MAX_TOKEN_BUDGET: usize = 1_000_000;
const MAX_VALUE_BYTES: usize = 4096;

struct TargetPreparation {
    kind: RouteKind,
    target_reference: String,
    next_action: &'static str,
    admission: Vec<u8>,
    delivery_schema: &'static str,
}

struct PreparedPaths<'a> {
    context: &'a Path,
    review: &'a Path,
    egress: &'a Path,
    admission: &'a Path,
    delivery: &'a Path,
    delivery_output: &'a Path,
}

struct PreparedBytes<'a> {
    context: &'a [u8],
    review: &'a [u8],
    egress: &'a [u8],
    admission: &'a [u8],
    delivery: &'a [u8],
}

#[derive(Clone, Copy)]
enum RouteKind {
    Managed,
    Delegated,
}

#[derive(Serialize)]
struct Artifact<'a> {
    path: String,
    hash: &'a str,
}

pub(crate) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let request_bytes = bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "Slice review preparation request",
    )?;
    let mut request: Request = serde_json::from_slice(&request_bytes).map_err(|error| {
        format!(
            "invalid Slice review preparation request {}: {error}",
            request_path.display()
        )
    })?;
    validate_and_normalize(&mut request)?;

    let repository = slice_worktree::repository_root(repository)?;
    let workspace = slice_worktree::workspace_root(&repository)?;
    let bound = slice_contract::trusted_bound_slice(&repository)?;
    if request.slice != bound.slice {
        return Err(format!(
            "review preparation names Slice `{}`, but the worktree is bound to `{}`",
            request.slice, bound.slice
        ));
    }
    let shared_directory = workspace
        .join(".local-exclude/coordination")
        .join(&request.slice);
    let expected_contract = shared_directory.join("slice-contract.json");
    let expected_contract = std::fs::canonicalize(&expected_contract).map_err(|error| {
        format!(
            "cannot resolve standard Slice contract {}: {error}",
            expected_contract.display()
        )
    })?;
    if expected_contract != bound.contract_path {
        return Err(
            "review preparation requires the bound contract at the standard coordination path"
                .to_owned(),
        );
    }
    bounded_file::ensure_directory(&shared_directory, "Slice review coordination")?;

    let local_directory = repository
        .join(".local-exclude/coordination")
        .join(&request.slice);
    bounded_file::ensure_directory(&local_directory, "candidate review preparation")?;
    let context_path = local_directory.join("context-request.json");
    let review_path = local_directory.join("review-request.json");
    let egress_path = shared_directory.join("review-egress.json");
    let admission_path = shared_directory.join("review-admission.json");
    let delivery_path = shared_directory.join("review-delivery.json");
    let output_directory = shared_directory.join("review-delivery");
    let context_relative = relative(&repository, &context_path);

    let context = ContextRequest {
        schema: CONTEXT_SCHEMA,
        anchors: request
            .knowledge_ids
            .iter()
            .map(|value| ContextAnchor {
                kind: "knowledge_id",
                value,
            })
            .collect(),
        tokenizer_profile: TOKENIZER_PROFILE,
        max_tokens: request.context_max_tokens,
    };
    let context_bytes = canonical_json(&context)?;
    let review = ReviewRequest {
        schema: REVIEW_SCHEMA,
        context_request_path: &context_relative,
        required_knowledge_ids: &request.knowledge_ids,
        slice_contract_path: bound.contract_path.to_string_lossy().into_owned(),
        repository_authority_paths: &request.repository_authority_paths,
        validation_evidence: &request.validation_evidence,
        review_lenses: &request.review_lenses,
        review_questions: &request.review_questions,
        delivery_profile: DELIVERY_PROFILE,
        tokenizer_profile: TOKENIZER_PROFILE,
        max_managed_payload_tokens: request.max_managed_payload_tokens,
    };
    let review_bytes = canonical_json(&review)?;

    let mut created = false;
    created |= publish(&context_path, &context_bytes, "ContextBuild request")?;
    created |= publish(&review_path, &review_bytes, "Slice review packet request")?;
    let target = target_preparation(&request.target)?;
    created |= publish(
        &admission_path,
        &target.admission,
        "review target admission request",
    )?;
    require_eligible_admission(&admission_path, target.next_action)?;
    require_exact(
        &admission_path,
        &target.admission,
        "review target admission request",
    )?;
    require_current(request_path, &request_bytes)?;

    let published = review_packet::publish(&repository, &review_path)?;
    created |= published.status == "created";
    require_current(request_path, &request_bytes)?;

    let egress = egress_document(&workspace, &request.target, &published)?;
    let egress_relative = relative(&workspace, &egress_path);
    let admission_relative = relative(&workspace, &admission_path);
    let output_relative = relative(&workspace, &output_directory);

    created |= publish(&egress_path, &egress, "review egress request")?;
    require_exact(&egress_path, &egress, "review egress request")?;
    require_exact(
        &admission_path,
        &target.admission,
        "review target admission request",
    )?;
    require_current(request_path, &request_bytes)?;

    authorize_route(&repository, target.kind, &egress_path, &published)?;
    require_eligible_admission(&admission_path, target.next_action)?;
    require_current(request_path, &request_bytes)?;

    bounded_file::ensure_directory(&output_directory, "review delivery output")?;
    require_empty_directory(&output_directory)?;
    let egress_hash = digest(&egress);
    let admission_hash = digest(&target.admission);
    let delivery = DeliveryRequest {
        schema: target.delivery_schema,
        egress_request_path: &egress_relative,
        egress_request_hash: &egress_hash,
        admission_request_path: &admission_relative,
        admission_request_hash: &admission_hash,
        output_directory: &output_relative,
    };
    let delivery_bytes = canonical_json(&delivery)?;
    created |= publish(&delivery_path, &delivery_bytes, "review delivery request")?;
    final_revalidate(
        &repository,
        &workspace,
        request_path,
        &request_bytes,
        &request.target,
        &target,
        &published,
        &bound,
        PreparedPaths {
            context: &context_path,
            review: &review_path,
            egress: &egress_path,
            admission: &admission_path,
            delivery: &delivery_path,
            delivery_output: &output_directory,
        },
        PreparedBytes {
            context: &context_bytes,
            review: &review_bytes,
            egress: &egress,
            admission: &target.admission,
            delivery: &delivery_bytes,
        },
    )?;

    let context_hash = digest(&context_bytes);
    let review_hash = digest(&review_bytes);
    let delivery_hash = digest(&delivery_bytes);
    let result = serde_json::json!({
        "schema": RESULT_SCHEMA,
        "ok": true,
        "status": if created { "created" } else { "reused" },
        "artifacts_published": true,
        "provider_requests": 0,
        "slice": request.slice,
        "candidate_commit": published.candidate_commit,
        "review_id": published.review_id,
        "target": target.target_reference,
        "packet": {
            "path": published.packet_path,
            "hash": published.packet_hash,
            "bytes": published.packet_bytes,
            "managed_payload_tokens": published.managed_payload_tokens,
            "max_managed_payload_tokens": published.max_managed_payload_tokens
        },
        "manifest": {
            "path": published.manifest_path,
            "hash": published.manifest_hash
        },
        "requests": {
            "context": Artifact { path: context_path.to_string_lossy().into_owned(), hash: &context_hash },
            "review": Artifact { path: review_path.to_string_lossy().into_owned(), hash: &review_hash },
            "egress": Artifact { path: egress_path.to_string_lossy().into_owned(), hash: &egress_hash },
            "admission": Artifact { path: admission_path.to_string_lossy().into_owned(), hash: &admission_hash },
            "delivery": Artifact { path: delivery_path.to_string_lossy().into_owned(), hash: &delivery_hash }
        },
        "delivery_output_directory": output_directory,
        "next_action": target.next_action
    });
    println!(
        "{}",
        serde_json::to_string(&result)
            .map_err(|error| format!("cannot encode review preparation result: {error}"))?
    );
    Ok(())
}

fn target_preparation(target: &Target) -> Result<TargetPreparation, String> {
    match target {
        Target::ManagedModel {
            provider,
            account,
            model,
            connection_repository_path,
            session_repository_path,
        } => Ok(TargetPreparation {
            kind: RouteKind::Managed,
            target_reference: format!("{provider}:{account}:{model}"),
            next_action: "deliver_once",
            admission: canonical_json(&ManagedAdmission {
                schema: MANAGED_ADMISSION_SCHEMA,
                target: ManagedAdmissionTarget {
                    kind: "managed_model",
                    provider,
                    account,
                    model,
                },
                connection_repository_path,
                session_repository_path: session_repository_path.as_deref(),
            })?,
            delivery_schema: MANAGED_DELIVERY_SCHEMA,
        }),
        Target::DelegatedHost {
            host,
            session_repository_path,
        } => Ok(TargetPreparation {
            kind: RouteKind::Delegated,
            target_reference: format!("host:{host}"),
            next_action: "deliver_delegated_once",
            admission: canonical_json(&DelegatedAdmission {
                schema: DELEGATED_ADMISSION_SCHEMA,
                target: DelegatedTarget {
                    kind: "delegated_host",
                    host,
                },
                session_repository_path: session_repository_path.as_deref(),
            })?,
            delivery_schema: DELEGATED_DELIVERY_SCHEMA,
        }),
    }
}

fn egress_document(
    workspace: &Path,
    target: &Target,
    published: &review_packet::PublishedReview,
) -> Result<Vec<u8>, String> {
    let fresh = FreshSession { mode: "fresh" };
    match target {
        Target::ManagedModel {
            provider,
            account,
            model,
            ..
        } => {
            let authorization =
                workspace.join(".local-exclude/authorizations/external-review.json");
            let authorization_bytes = bounded_file::read_regular(
                &authorization,
                AUTHORIZATION_LIMIT,
                "managed external-review authorization",
            )?;
            let authorization_hash = digest(&authorization_bytes);
            canonical_json(&ManagedEgress {
                schema: MANAGED_EGRESS_SCHEMA,
                manifest_path: &published.manifest_path,
                manifest_hash: &published.manifest_hash,
                authorization_hash: &authorization_hash,
                route: ManagedRoute {
                    provider,
                    account,
                    model,
                },
                session: fresh,
            })
        },
        Target::DelegatedHost { host, .. } => {
            let authorization =
                workspace.join(".local-exclude/authorizations/external-review-delegated.json");
            let authorization_bytes = bounded_file::read_regular(
                &authorization,
                AUTHORIZATION_LIMIT,
                "delegated external-review authorization",
            )?;
            let authorization_hash = digest(&authorization_bytes);
            canonical_json(&DelegatedEgress {
                schema: DELEGATED_EGRESS_SCHEMA,
                manifest_path: &published.manifest_path,
                manifest_hash: &published.manifest_hash,
                authorization_hash: &authorization_hash,
                target: DelegatedTarget {
                    kind: "delegated_host",
                    host,
                },
                execution_profile: DELEGATED_EXECUTION_PROFILE,
                session: fresh,
            })
        },
    }
}

fn authorize_route(
    repository: &Path,
    kind: RouteKind,
    egress_path: &Path,
    published: &review_packet::PublishedReview,
) -> Result<(), String> {
    let (review_id, candidate_commit) = match kind {
        RouteKind::Managed => {
            let authorized = review_egress::authorize_delivery(repository, egress_path)?;
            (authorized.review_id, authorized.candidate_commit)
        },
        RouteKind::Delegated => {
            let authorized = review_egress::authorize_host_delivery(repository, egress_path)?;
            (authorized.review_id, authorized.candidate_commit)
        },
    };
    if review_id != published.review_id || candidate_commit != published.candidate_commit {
        return Err("prepared egress does not authorize the published candidate review".to_owned());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn final_revalidate(
    repository: &Path,
    workspace: &Path,
    request_path: &Path,
    request_bytes: &[u8],
    target: &Target,
    prepared_target: &TargetPreparation,
    published: &review_packet::PublishedReview,
    bound: &slice_contract::BoundSlice,
    paths: PreparedPaths<'_>,
    bytes: PreparedBytes<'_>,
) -> Result<(), String> {
    require_current(request_path, request_bytes)?;
    require_prepared_requests_current(&paths, &bytes)?;
    require_empty_directory(paths.delivery_output)?;
    require_same_bound_slice(repository, bound)?;
    slice_worktree::ensure_clean(
        repository,
        "candidate worktree",
        "returning review preparation",
    )?;
    if slice_worktree::resolve_commit(repository, "HEAD")? != published.candidate_commit {
        return Err("candidate HEAD changed during review preparation".to_owned());
    }
    if slice_worktree::resolve_commit(repository, "refs/heads/develop")? != published.trusted_commit
    {
        return Err("trusted integration changed during review preparation".to_owned());
    }

    let current_egress = egress_document(workspace, target, published)?;
    if current_egress != bytes.egress {
        return Err("review authorization changed during review preparation".to_owned());
    }
    authorize_route(repository, prepared_target.kind, paths.egress, published)?;
    require_eligible_admission(paths.admission, prepared_target.next_action)
}

fn require_prepared_requests_current(
    paths: &PreparedPaths<'_>,
    bytes: &PreparedBytes<'_>,
) -> Result<(), String> {
    require_exact(paths.context, bytes.context, "ContextBuild request")?;
    require_exact(paths.review, bytes.review, "Slice review packet request")?;
    require_exact(paths.egress, bytes.egress, "review egress request")?;
    require_exact(
        paths.admission,
        bytes.admission,
        "review target admission request",
    )?;
    require_exact(paths.delivery, bytes.delivery, "review delivery request")
}

fn require_eligible_admission(path: &Path, expected_next_action: &str) -> Result<(), String> {
    let admission = review_target_admission::evaluate(path)?;
    let value = serde_json::to_value(admission)
        .map_err(|error| format!("cannot inspect review admission result: {error}"))?;
    let ok = value.get("ok").and_then(serde_json::Value::as_bool) == Some(true);
    let next_action = value
        .get("next_action")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    if !ok || next_action != expected_next_action {
        return Err(format!(
            "review target admission stopped preparation with next action `{next_action}`"
        ));
    }
    Ok(())
}

fn validate_and_normalize(request: &mut Request) -> Result<(), String> {
    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "unsupported Slice review preparation schema `{}`; expected `{REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    compact_token(&request.slice, "slice")?;
    require_path_component(&request.slice, "slice")?;
    normalize_strings(&mut request.knowledge_ids, "knowledge_ids")?;
    normalize_strings(
        &mut request.repository_authority_paths,
        "repository_authority_paths",
    )?;
    normalize_strings(&mut request.review_lenses, "review_lenses")?;
    require_non_empty_strings(&request.review_questions, "review_questions")?;
    if request.validation_evidence.is_empty() {
        return Err("validation_evidence must not be empty".to_owned());
    }
    request
        .validation_evidence
        .sort_by(|left, right| left.name.cmp(&right.name));
    let mut names = BTreeSet::new();
    for evidence in &request.validation_evidence {
        compact_token(&evidence.name, "validation evidence name")?;
        compact_value(&evidence.path, "validation evidence path")?;
        if !names.insert(evidence.name.as_str()) {
            return Err(format!(
                "validation_evidence contains duplicate name `{}`",
                evidence.name
            ));
        }
    }
    for path in &request.repository_authority_paths {
        require_repository_relative(path, "repository authority path")?;
    }
    require_budget(request.context_max_tokens, "context_max_tokens")?;
    require_budget(
        request.max_managed_payload_tokens,
        "max_managed_payload_tokens",
    )?;
    match &request.target {
        Target::ManagedModel {
            provider,
            account,
            model,
            connection_repository_path,
            session_repository_path,
        } => {
            compact_token(provider, "target provider")?;
            compact_token(account, "target account")?;
            compact_token(model, "target model")?;
            require_absolute(connection_repository_path, "connection_repository_path")?;
            if let Some(path) = session_repository_path {
                require_absolute(path, "session_repository_path")?;
            }
        },
        Target::DelegatedHost {
            host,
            session_repository_path,
        } => {
            if !matches!(host.as_str(), "codex" | "grok") {
                return Err("delegated review target must be `codex` or `grok`".to_owned());
            }
            if let Some(path) = session_repository_path {
                require_absolute(path, "session_repository_path")?;
            }
        },
    }
    Ok(())
}

fn normalize_strings(values: &mut [String], label: &str) -> Result<(), String> {
    require_non_empty_strings(values, label)?;
    values.sort();
    for pair in values.windows(2) {
        if pair[0] == pair[1] {
            return Err(format!("{label} contains duplicate value `{}`", pair[0]));
        }
    }
    Ok(())
}

fn require_non_empty_strings(values: &[String], label: &str) -> Result<(), String> {
    if values.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    for value in values {
        compact_value(value, label)?;
    }
    Ok(())
}

fn compact_token(value: &str, label: &str) -> Result<(), String> {
    compact_value(value, label)?;
    if value
        .bytes()
        .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(format!("{label} must be one compact token"));
    }
    Ok(())
}

fn compact_value(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value != value.trim() || value.len() > MAX_VALUE_BYTES {
        return Err(format!(
            "{label} must be non-empty, trimmed, and at most {MAX_VALUE_BYTES} bytes"
        ));
    }
    Ok(())
}

fn require_repository_relative(value: &str, label: &str) -> Result<(), String> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{label} must be a normalized repository-relative path"
        ));
    }
    Ok(())
}

fn require_path_component(value: &str, label: &str) -> Result<(), String> {
    let mut components = Path::new(value).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(format!("{label} must be one normalized path component"));
    }
    Ok(())
}

fn require_absolute(value: &str, label: &str) -> Result<(), String> {
    compact_value(value, label)?;
    let path = Path::new(value);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(format!("{label} must be a normalized absolute path"));
    }
    Ok(())
}

fn require_budget(value: usize, label: &str) -> Result<(), String> {
    if value == 0 || value > MAX_TOKEN_BUDGET {
        return Err(format!("{label} must be between 1 and {MAX_TOKEN_BUDGET}"));
    }
    Ok(())
}

fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("cannot encode prepared review request: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn publish(path: &Path, bytes: &[u8], label: &str) -> Result<bool, String> {
    bounded_file::publish_new_or_exact(path, bytes, GENERATED_REQUEST_LIMIT, label)
}

fn require_exact(path: &Path, expected: &[u8], label: &str) -> Result<(), String> {
    let current = bounded_file::read_regular(path, GENERATED_REQUEST_LIMIT, label)?;
    if current == expected {
        Ok(())
    } else {
        Err(format!("{label} changed during review preparation"))
    }
}

fn require_current(path: &Path, expected: &[u8]) -> Result<(), String> {
    let current =
        bounded_file::read_regular(path, REQUEST_LIMIT, "Slice review preparation request")?;
    if current == expected {
        Ok(())
    } else {
        Err("Slice review preparation request changed during preparation".to_owned())
    }
}

fn require_same_bound_slice(
    repository: &Path,
    expected: &slice_contract::BoundSlice,
) -> Result<(), String> {
    let current = slice_contract::trusted_bound_slice(repository)?;
    if &current == expected {
        Ok(())
    } else {
        Err("Slice binding or contract changed during review preparation".to_owned())
    }
}

fn require_empty_directory(path: &Path) -> Result<(), String> {
    let directory = bounded_file::open_directory(path, "review delivery output")?;
    let mut entries = Dir::read_from(&directory)
        .map_err(|error| format!("cannot enumerate review delivery output: {error}"))?;
    while let Some(entry) = entries.read() {
        let entry =
            entry.map_err(|error| format!("cannot enumerate review delivery output: {error}"))?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            return Err(
                "review delivery output is not empty; inspect its existing claim or result"
                    .to_owned(),
            );
        }
    }
    Ok(())
}
