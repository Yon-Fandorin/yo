use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{Observation, REQUEST_LIMIT};
use crate::{
    bounded_file, review_egress::AuthorizedDelivery,
    review_session::provider_request_identity as session_provider_request_identity,
};

pub(super) fn require_finding_resolution(
    delivery: &AuthorizedDelivery,
) -> Result<(&str, &str, &str), String> {
    if delivery.review_kind != "finding_resolution" || delivery.fresh_session {
        return Err(
            "continuation preflight accepts only an authorized finding-resolution resume"
                .to_owned(),
        );
    }
    let session_id = delivery
        .session_id
        .as_deref()
        .ok_or_else(|| "finding-resolution authorization has no reviewer Session".to_owned())?;
    let packet_hash = delivery.prior_packet_hash.as_deref().ok_or_else(|| {
        "finding-resolution authorization has no prior original packet hash".to_owned()
    })?;
    let request_id = delivery
        .prior_provider_request_id
        .as_deref()
        .ok_or_else(|| {
            "finding-resolution authorization has no prior Provider request identity".to_owned()
        })?;
    Ok((session_id, packet_hash, request_id))
}

pub(super) fn resolve_session_root(repository: &Path, requested: &str) -> Result<PathBuf, String> {
    let requested = PathBuf::from(requested);
    let requested = if requested.is_absolute() {
        requested
    } else {
        repository.join(requested)
    };
    let metadata = fs::symlink_metadata(&requested).map_err(|error| {
        format!(
            "cannot inspect reviewer Session repository {}: {error}",
            requested.display()
        )
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("reviewer Session repository must be a real directory".to_owned());
    }
    fs::canonicalize(&requested).map_err(|error| {
        format!(
            "cannot resolve reviewer Session repository {}: {error}",
            requested.display()
        )
    })
}

pub(super) fn validate_observation(
    observation: &Observation,
    expected_packet_hash: &str,
    expected_provider_request_id: &str,
) -> Result<(), String> {
    if observation.start_packet_hashes != [expected_packet_hash] {
        return Err(
            "reviewer Session does not contain exactly one original StartTurn matching the prior immutable packet"
                .to_owned(),
        );
    }
    if observation.binding_matches != [true] {
        return Err(
            "reviewer Session does not contain exactly one matching Provider/Account/Model binding"
                .to_owned(),
        );
    }
    let observed = session_provider_request_identity(
        &observation.request_identities,
        &observation.outcome_identities,
    )?;
    if observed != expected_provider_request_id {
        return Err(
            "reviewer Session Provider request identity differs from the prior delivery receipt"
                .to_owned(),
        );
    }
    if observation.continuation_anchors.len() != 1 {
        return Err(format!(
            "reviewer Session contains {} durable Continuation Anchors instead of exactly one",
            observation.continuation_anchors.len()
        ));
    }
    Ok(())
}

pub(super) fn require_unchanged_file(
    path: &Path,
    expected: &[u8],
    label: &str,
) -> Result<(), String> {
    let actual = bounded_file::read_regular(path, REQUEST_LIMIT, label)?;
    if actual != expected {
        return Err(format!("{label} changed during continuation preflight"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{require_finding_resolution, validate_observation};
    use crate::{review_continuation_preflight::Observation, review_egress::AuthorizedDelivery};

    fn authorized() -> AuthorizedDelivery {
        AuthorizedDelivery {
            request_id: "sha256:request".to_owned(),
            authorization_id: "sha256:authorization".to_owned(),
            authority: "human/yon".to_owned(),
            review_kind: "finding_resolution",
            review_id: "sha256:review".to_owned(),
            candidate_commit: "11".repeat(20),
            trusted_commit: "22".repeat(20),
            packet_hash: "sha256:delta".to_owned(),
            packet_bytes: b"delta".to_vec(),
            managed_payload_tokens: 2,
            provider: "kimi".to_owned(),
            account: "default".to_owned(),
            model: "k3-256k".to_owned(),
            fresh_session: false,
            session_id: Some("01890f00-0000-7000-8000-000000000001".to_owned()),
            prior_packet_hash: Some("sha256:prior".to_owned()),
            prior_provider_request_id: Some("request-1".to_owned()),
        }
    }

    // original fresh 권한이나 identity가 빠진 resume은 Session filesystem을 열기 전에
    // 거부되어 terminal input 후보가 되지 않습니다.
    #[test]
    fn only_complete_finding_resolution_authority_reaches_session_inspection() {
        require_finding_resolution(&authorized()).unwrap();
        let mut original = authorized();
        original.review_kind = "original";
        original.fresh_session = true;
        assert!(require_finding_resolution(&original).is_err());

        let mut missing = authorized();
        missing.prior_provider_request_id = None;
        assert!(
            require_finding_resolution(&missing)
                .unwrap_err()
                .contains("request identity")
        );
    }

    // exact original packet, route, request와 outcome identity가 모두 일치해야만 typed
    // Continuation Anchor 검사로 진행할 수 있습니다.
    #[test]
    fn observation_is_bound_to_one_original_request_and_route() {
        let valid = Observation {
            start_packet_hashes: vec!["sha256:prior".to_owned()],
            binding_matches: vec![true],
            request_identities: vec!["request-1".to_owned()],
            outcome_identities: vec![None],
            continuation_anchors: vec![(1, 5, 7, 7)],
        };
        validate_observation(&valid, "sha256:prior", "request-1").unwrap();

        let mut wrong_packet = Observation { ..valid };
        wrong_packet.start_packet_hashes = vec!["sha256:other".to_owned()];
        assert!(
            validate_observation(&wrong_packet, "sha256:prior", "request-1")
                .unwrap_err()
                .contains("StartTurn")
        );

        let wrong_route = Observation {
            start_packet_hashes: vec!["sha256:prior".to_owned()],
            binding_matches: vec![false],
            request_identities: vec!["request-1".to_owned()],
            outcome_identities: vec![None],
            continuation_anchors: vec![(1, 5, 7, 7)],
        };
        assert!(
            validate_observation(&wrong_route, "sha256:prior", "request-1")
                .unwrap_err()
                .contains("Provider/Account/Model")
        );

        let wrong_request = Observation {
            start_packet_hashes: vec!["sha256:prior".to_owned()],
            binding_matches: vec![true],
            request_identities: vec!["request-2".to_owned()],
            outcome_identities: vec![None],
            continuation_anchors: vec![(1, 5, 7, 7)],
        };
        assert!(
            validate_observation(&wrong_request, "sha256:prior", "request-1")
                .unwrap_err()
                .contains("differs")
        );

        let missing_anchor = Observation {
            start_packet_hashes: vec!["sha256:prior".to_owned()],
            binding_matches: vec![true],
            request_identities: vec!["request-1".to_owned()],
            outcome_identities: vec![None],
            continuation_anchors: Vec::new(),
        };
        assert!(
            validate_observation(&missing_anchor, "sha256:prior", "request-1")
                .unwrap_err()
                .contains("Continuation Anchor")
        );
    }
}
