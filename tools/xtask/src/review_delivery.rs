mod admission;
mod artifact;
mod continuation;
mod delegated;
mod delegated_session;
mod finalize;
mod model;
mod original;
mod process;
mod runner_capability;
mod session;
mod usage;
mod workspace;

#[cfg(test)]
mod tests;

use std::path::Path;

use model::DeliveryRequest;

const REQUEST_LIMIT: usize = 64 * 1024;
const REVIEW_RESULT_LIMIT: usize = 4 * 1024 * 1024;
const DIAGNOSTIC_LIMIT: usize = 256 * 1024;
const REQUEST_SCHEMA_V1_ALPHA3: &str = "yo.slice-review-delivery-request/v1alpha3";
const REQUEST_SCHEMA_V1_ALPHA4: &str = "yo.slice-review-delivery-request/v1alpha4";
const CONTINUATION_REQUEST_SCHEMA_V1_ALPHA3: &str =
    "yo.slice-review-continuation-delivery-request/v1alpha3";
const CONTINUATION_REQUEST_SCHEMA_V1_ALPHA4: &str =
    "yo.slice-review-continuation-delivery-request/v1alpha4";
const DELEGATED_REQUEST_SCHEMA_V1_ALPHA3: &str =
    "yo.slice-review-delegated-delivery-request/v1alpha3";
const DELEGATED_REQUEST_SCHEMA_V1_ALPHA4: &str =
    "yo.slice-review-delegated-delivery-request/v1alpha4";
const DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA3: &str =
    "yo.slice-review-delegated-continuation-delivery-request/v1alpha3";
const DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA4: &str =
    "yo.slice-review-delegated-continuation-delivery-request/v1alpha4";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct DeliveryPolicy {
    prepare_output: bool,
    bind_usage: bool,
}

pub(crate) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let (request, policy) = admission::read_request_with_output_policy(request_path)?;
    match request {
        DeliveryRequest::Original(request) => original::run(repository, request, None, policy),
        DeliveryRequest::AdmittedOriginal(request) => {
            let admission = admission::AdmissionReference {
                path: request.admission_request_path,
                hash: request.admission_request_hash,
            };
            original::run(
                repository,
                model::Request {
                    schema: request.schema,
                    egress_request_path: request.egress_request_path,
                    egress_request_hash: request.egress_request_hash,
                    output_directory: request.output_directory,
                },
                Some(admission),
                policy,
            )
        },
        DeliveryRequest::Continuation(request) => {
            continuation::run(repository, request, None, policy)
        },
        DeliveryRequest::AdmittedContinuation(request) => {
            let admission = admission::AdmissionReference {
                path: request.admission_request_path,
                hash: request.admission_request_hash,
            };
            continuation::run(
                repository,
                model::ContinuationRequest {
                    schema: request.schema,
                    preflight_request_path: request.preflight_request_path,
                    preflight_request_hash: request.preflight_request_hash,
                    output_directory: request.output_directory,
                },
                Some(admission),
                policy,
            )
        },
        DeliveryRequest::Delegated(request) => delegated::run_original(repository, request, policy),
        DeliveryRequest::DelegatedContinuation(request) => {
            delegated::run_continuation(repository, request, policy)
        },
    }
}

pub(crate) fn finalize(repository: &Path, request_path: &Path) -> Result<(), String> {
    finalize::run(repository, request_path)
}
