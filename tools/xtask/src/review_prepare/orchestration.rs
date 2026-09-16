use std::{fs, path::Path};

use super::{
    authority::apply_repository_authority_policy,
    files::{
        PreparedBytes, PreparedPaths, REQUEST_LIMIT, canonical_json, publish, require_current,
        require_empty_directory, require_exact,
    },
    model::{
        Artifact, CONTEXT_SCHEMA, ContextAnchor, ContextRequest, DELIVERY_PROFILE, DeliveryRequest,
        REQUEST_SCHEMA, REQUEST_SCHEMA_V1_ALPHA2, REQUEST_SCHEMA_V1_ALPHA3,
        REQUEST_SCHEMA_V1_ALPHA4, REQUEST_SCHEMA_V1_ALPHA5, REQUEST_SCHEMA_V1_ALPHA6,
        REQUEST_SCHEMA_V1_ALPHA7, RESULT_SCHEMA, RESULT_SCHEMA_V1_ALPHA2, RESULT_SCHEMA_V1_ALPHA3,
        RESULT_SCHEMA_V1_ALPHA4, RESULT_SCHEMA_V1_ALPHA5, RESULT_SCHEMA_V1_ALPHA6,
        RESULT_SCHEMA_V1_ALPHA7, REVIEW_SCHEMA, Request, ReviewRequest, TOKENIZER_PROFILE,
    },
    request::{prepared_review_questions, validate_and_normalize},
    revalidation::{final_revalidate, require_eligible_admission},
    target::{authorize_route, egress_document, target_preparation},
};
use crate::{
    bounded_file, review_packet,
    review_protocol::{digest, relative},
    slice_contract, slice_worktree,
};

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
    apply_repository_authority_policy(&repository, &bound, &mut request)?;
    let shared_directory = workspace
        .join(".local-exclude/coordination")
        .join(&request.slice);
    let expected_contract = shared_directory.join("slice-contract.json");
    let expected_contract = fs::canonicalize(&expected_contract).map_err(|error| {
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
    let review_questions = prepared_review_questions(&request);
    let review = ReviewRequest {
        schema: REVIEW_SCHEMA,
        context_request_path: &context_relative,
        required_knowledge_ids: &request.knowledge_ids,
        slice_contract_path: bound.contract_path.to_string_lossy().into_owned(),
        repository_authority_paths: &request.repository_authority_paths,
        validation_evidence: &request.validation_evidence,
        review_lenses: &request.review_lenses,
        review_questions: &review_questions,
        delivery_profile: DELIVERY_PROFILE,
        tokenizer_profile: TOKENIZER_PROFILE,
        max_managed_payload_tokens: request.max_managed_payload_tokens,
    };
    let review_bytes = canonical_json(&review)?;

    let mut created = false;
    created |= publish(&context_path, &context_bytes, "ContextBuild request")?;
    created |= publish(&review_path, &review_bytes, "Slice review packet request")?;
    let target = target_preparation(
        &request.target,
        matches!(
            request.schema.as_str(),
            REQUEST_SCHEMA_V1_ALPHA3
                | REQUEST_SCHEMA_V1_ALPHA4
                | REQUEST_SCHEMA_V1_ALPHA5
                | REQUEST_SCHEMA_V1_ALPHA6
                | REQUEST_SCHEMA_V1_ALPHA7
        ),
        matches!(
            request.schema.as_str(),
            REQUEST_SCHEMA_V1_ALPHA6 | REQUEST_SCHEMA_V1_ALPHA7
        ),
        request.schema == REQUEST_SCHEMA_V1_ALPHA7,
    )?;
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
    let result_schema = match request.schema.as_str() {
        REQUEST_SCHEMA_V1_ALPHA7 => RESULT_SCHEMA_V1_ALPHA7,
        REQUEST_SCHEMA_V1_ALPHA6 => RESULT_SCHEMA_V1_ALPHA6,
        REQUEST_SCHEMA_V1_ALPHA5 => RESULT_SCHEMA_V1_ALPHA5,
        REQUEST_SCHEMA_V1_ALPHA4 => RESULT_SCHEMA_V1_ALPHA4,
        REQUEST_SCHEMA_V1_ALPHA3 => RESULT_SCHEMA_V1_ALPHA3,
        REQUEST_SCHEMA_V1_ALPHA2 => RESULT_SCHEMA_V1_ALPHA2,
        _ => RESULT_SCHEMA,
    };
    let result = serde_json::json!({
        "schema": result_schema,
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
