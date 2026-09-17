use std::path::PathBuf;

use jiff::Timestamp;
use yo_core::{LocalConnectionRepository, ModelLastFailure, ModelRequestFailureKind};

use super::model::Availability;

pub(super) const BLOCKING_FAILURE_FRESHNESS_SECONDS: i64 = 5 * 60 * 60;

pub(super) fn managed_availability(
    path: &str,
    provider: &str,
    account: &str,
    model: &str,
    expire_blocking_failure: bool,
    now: Timestamp,
) -> Availability {
    let repository = LocalConnectionRepository::new(PathBuf::from(path));
    let snapshot = match repository.capture() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Availability {
                state: "unavailable",
                source: "connection_repository",
                failure_kind: Some("local_configuration".to_owned()),
                observed_at: None,
                failure_freshness: None,
                version: None,
                executable: None,
                execution_isolation: None,
                detail: format!("cannot read the exact connection repository: {error}"),
            };
        },
    };
    let selected = snapshot.models().iter().find(|stored| {
        let binding = stored.complete().binding();
        binding.provider_id().as_str() == provider
            && binding.account_id().as_str() == account
            && binding.model_id().as_str() == model
    });
    let Some(selected) = selected else {
        return Availability {
            state: "unavailable",
            source: "connection_repository",
            failure_kind: Some("local_configuration".to_owned()),
            observed_at: None,
            failure_freshness: None,
            version: None,
            executable: None,
            execution_isolation: None,
            detail: "the exact managed review target is not stored".to_owned(),
        };
    };
    let Some(failure) = selected.last_failure() else {
        return Availability {
            state: "unknown",
            source: "connection_repository",
            failure_kind: None,
            observed_at: None,
            failure_freshness: None,
            version: None,
            executable: None,
            execution_isolation: None,
            detail:
                "the binding is stored, but no request-free entitlement or quota proof is available"
                    .to_owned(),
        };
    };
    observed_failure_availability(failure, expire_blocking_failure, now)
}

pub(super) fn observed_failure_availability(
    failure: &ModelLastFailure,
    expire_blocking_failure: bool,
    now: Timestamp,
) -> Availability {
    let blocking = blocking_failure(failure.kind());
    let observed = failure
        .observed_at()
        .parse::<Timestamp>()
        .expect("stored last_failure was validated before repository capture");
    let stale =
        now.as_second().saturating_sub(observed.as_second()) >= BLOCKING_FAILURE_FRESHNESS_SECONDS;
    let stale_blocking = expire_blocking_failure && blocking && stale;
    Availability {
        state: if blocking && !stale_blocking {
            "unavailable"
        } else {
            "unknown"
        },
        source: "connection_repository.last_failure",
        failure_kind: Some(failure.kind().as_str().to_owned()),
        observed_at: Some(failure.observed_at().to_owned()),
        failure_freshness: expire_blocking_failure.then_some(if stale {
            "stale"
        } else {
            "recent"
        }),
        version: None,
        executable: None,
        execution_isolation: None,
        detail: if stale_blocking {
            "the blocking observation is at least 5 hours old; the already-authorized original delivery may revalidate it exactly once without a probe, retry, steer, or fallback"
                .to_owned()
        } else if blocking {
            "the newest typed target observation is unavailable; a successful exact request must clear it"
                .to_owned()
        } else {
            "the newest typed failure is reported without inferring quota exhaustion or current unavailability"
                .to_owned()
        },
    }
}

pub(super) const fn blocking_failure(kind: ModelRequestFailureKind) -> bool {
    matches!(
        kind,
        ModelRequestFailureKind::Authentication
            | ModelRequestFailureKind::AccessDenied
            | ModelRequestFailureKind::ModelUnavailable
            | ModelRequestFailureKind::LocalConfiguration
    )
}
