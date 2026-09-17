mod delegated;
mod managed;
mod model;
mod request;
mod usage;

#[cfg(test)]
mod tests;

use std::path::Path;

use jiff::Timestamp;
use model::{
    AccountLimit, Decision, REQUEST_SCHEMA, REQUEST_SCHEMA_V1_ALPHA2, REQUEST_SCHEMA_V1_ALPHA3,
    REQUEST_SCHEMA_V1_ALPHA4, REQUEST_SCHEMA_V1_ALPHA5, REQUEST_SCHEMA_V1_ALPHA6, Request,
    result_schema,
};
pub(crate) use model::{Admission, ReviewTarget};

pub(crate) fn run(request_path: &Path) -> Result<(), String> {
    let admission = evaluate(request_path)?;
    println!(
        "{}",
        serde_json::to_string(&admission)
            .map_err(|error| format!("cannot encode review-target admission result: {error}"))?
    );
    Ok(())
}

pub(crate) fn evaluate(request_path: &Path) -> Result<Admission, String> {
    let request = request::read(request_path)?;
    evaluate_request(&request)
}

fn evaluate_request(request: &Request) -> Result<Admission, String> {
    let availability = match &request.target {
        ReviewTarget::ManagedModel {
            provider,
            account,
            model,
        } => managed::managed_availability(
            request
                .connection_repository_path
                .as_deref()
                .expect("validated managed request has a connection repository"),
            provider,
            account,
            model,
            request.schema == REQUEST_SCHEMA_V1_ALPHA6,
            Timestamp::now(),
        ),
        ReviewTarget::DelegatedHost { host } => delegated::host_availability(
            host,
            match request.schema.as_str() {
                REQUEST_SCHEMA_V1_ALPHA5 | REQUEST_SCHEMA_V1_ALPHA6 => {
                    delegated::HostReadiness::ExecutionIsolation
                },
                REQUEST_SCHEMA_V1_ALPHA4 => delegated::HostReadiness::ExecutionProfile,
                REQUEST_SCHEMA_V1_ALPHA3 => delegated::HostReadiness::State,
                REQUEST_SCHEMA | REQUEST_SCHEMA_V1_ALPHA2 => delegated::HostReadiness::Version,
                _ => unreachable!("validated admission schema"),
            },
        ),
    };
    let decision = if availability.state == "unavailable" {
        Decision::Stop
    } else {
        Decision::Admit
    };
    let (status, next_action) = if matches!(decision, Decision::Admit) {
        request.target.admitted_outcome(&request.schema)
    } else {
        ("stopped", "select_human_authorized_alternative")
    };
    let (usage_search, last_exact_usage_receipt) =
        usage::latest_receipt(request.session_repository_path.as_deref(), &request.target);
    Ok(Admission {
        schema: result_schema(&request.schema),
        ok: matches!(decision, Decision::Admit),
        status,
        next_action,
        decision,
        target_reference: request.target.reference(),
        target: request.target.clone(),
        availability,
        account_limit: AccountLimit {
            availability: "unknown",
            remaining: None,
            resets_at: None,
            source: None,
        },
        usage_search,
        last_exact_usage_receipt,
    })
}
