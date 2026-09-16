use std::path::Path;

use super::{
    files::{AUTHORIZATION_LIMIT, canonical_json},
    model::{
        DELEGATED_ADMISSION_SCHEMA, DELEGATED_DELIVERY_SCHEMA, DELEGATED_EGRESS_SCHEMA,
        DELEGATED_EXECUTION_PROFILE, DELEGATED_ISOLATION_ADMISSION_SCHEMA,
        DELEGATED_PROFILE_ADMISSION_SCHEMA, DELEGATED_USAGE_DELIVERY_SCHEMA, DelegatedAdmission,
        DelegatedEgress, DelegatedTarget, FreshSession, MANAGED_ADMISSION_SCHEMA,
        MANAGED_DELIVERY_SCHEMA, MANAGED_EGRESS_SCHEMA, MANAGED_FRESHNESS_ADMISSION_SCHEMA,
        MANAGED_USAGE_DELIVERY_SCHEMA, ManagedAdmission, ManagedAdmissionTarget, ManagedEgress,
        ManagedRoute, Target,
    },
};
use crate::{bounded_file, review_egress, review_packet, review_protocol::digest};

pub(super) struct TargetPreparation {
    pub(super) kind: RouteKind,
    pub(super) target_reference: String,
    pub(super) next_action: &'static str,
    pub(super) admission: Vec<u8>,
    pub(super) delivery_schema: &'static str,
}

#[derive(Clone, Copy)]
pub(super) enum RouteKind {
    Managed,
    Delegated,
}

pub(super) fn target_preparation(
    target: &Target,
    bind_usage: bool,
    probe_execution_profile: bool,
    use_current_admission: bool,
) -> Result<TargetPreparation, String> {
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
                schema: if use_current_admission {
                    MANAGED_FRESHNESS_ADMISSION_SCHEMA
                } else {
                    MANAGED_ADMISSION_SCHEMA
                },
                target: ManagedAdmissionTarget {
                    kind: "managed_model",
                    provider,
                    account,
                    model,
                },
                connection_repository_path,
                session_repository_path: session_repository_path.as_deref(),
            })?,
            delivery_schema: if bind_usage {
                MANAGED_USAGE_DELIVERY_SCHEMA
            } else {
                MANAGED_DELIVERY_SCHEMA
            },
        }),
        Target::DelegatedHost {
            host,
            session_repository_path,
        } => Ok(TargetPreparation {
            kind: RouteKind::Delegated,
            target_reference: format!("host:{host}"),
            next_action: "deliver_delegated_once",
            admission: canonical_json(&DelegatedAdmission {
                schema: if host == "grok" && use_current_admission {
                    DELEGATED_ISOLATION_ADMISSION_SCHEMA
                } else if host == "grok" && probe_execution_profile {
                    DELEGATED_PROFILE_ADMISSION_SCHEMA
                } else {
                    DELEGATED_ADMISSION_SCHEMA
                },
                target: DelegatedTarget {
                    kind: "delegated_host",
                    host,
                },
                session_repository_path: session_repository_path.as_deref(),
            })?,
            delivery_schema: if bind_usage {
                DELEGATED_USAGE_DELIVERY_SCHEMA
            } else {
                DELEGATED_DELIVERY_SCHEMA
            },
        }),
    }
}

pub(super) fn egress_document(
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

pub(super) fn authorize_route(
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
