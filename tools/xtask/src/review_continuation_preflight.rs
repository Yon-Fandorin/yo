use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use yo_core::{
    AgentCommand, SessionId, TranscriptRecord,
    session_repository::{
        LocalSessionReader, StoredRequestTraceRecord, read_stored_session,
        read_stored_session_continuation,
    },
};

use crate::{
    bounded_file,
    review_egress::{self, AuthorizedDelivery},
    review_protocol::{digest, resolve_input_path},
    review_session::{managed_binding_matches, provider_request_identity},
};

mod delegated;

pub(crate) use delegated::VerifiedHostContinuation;

pub(crate) fn evaluate_delegated(
    repository: &Path,
    request_path: &Path,
) -> Result<VerifiedHostContinuation, String> {
    delegated::evaluate(repository, request_path)
}

const REQUEST_SCHEMA: &str = "yo.slice-review-continuation-preflight-request/v1alpha1";
const RESULT_SCHEMA: &str = "yo.slice-review-continuation-preflight-result/v1alpha1";
const REQUEST_LIMIT: usize = 64 * 1024;
const MAX_PATH_BYTES: usize = 4096;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: String,
    egress_request_path: String,
    egress_request_hash: String,
    session_repository_path: String,
}

#[derive(Debug, Serialize)]
struct Route<'a> {
    provider: &'a str,
    account: &'a str,
    model: &'a str,
}

#[derive(Debug, Serialize)]
struct ResultDocument<'a> {
    schema: &'static str,
    ok: bool,
    status: &'static str,
    next_action: &'static str,
    artifacts_published: bool,
    provider_requests: usize,
    request_id: String,
    egress_request_id: &'a str,
    review_id: &'a str,
    candidate_commit: &'a str,
    session_id: &'a str,
    route: Route<'a>,
    prior_packet_hash: &'a str,
    prior_provider_request_id: &'a str,
    continuation_anchor_sequence: u64,
    binding_epoch: u64,
}

#[derive(Debug, Eq, PartialEq)]
struct Observation {
    start_packet_hashes: Vec<String>,
    binding_matches: Vec<bool>,
    request_identities: Vec<String>,
    outcome_identities: Vec<Option<String>>,
    continuation_anchors: Vec<(u64, u64, u64, u64)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedContinuation {
    pub(crate) preflight_request_id: String,
    pub(crate) delivery: AuthorizedDelivery,
    pub(crate) session_root: PathBuf,
    pub(crate) continuation_anchor_sequence: u64,
    pub(crate) binding_epoch: u64,
}

pub(crate) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let bytes = bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "Slice review continuation preflight request",
    )?;
    let schema = serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|error| format!("invalid continuation preflight request: {error}"))?
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "continuation preflight request has no string schema".to_owned())?
        .to_owned();
    if schema == delegated::REQUEST_SCHEMA {
        return delegated::run(repository, request_path);
    }
    let verified = evaluate(repository, request_path)?;
    let delivery = &verified.delivery;
    let (session_id, prior_packet_hash, prior_provider_request_id) =
        require_finding_resolution(delivery)?;
    let result = ResultDocument {
        schema: RESULT_SCHEMA,
        ok: true,
        status: "eligible",
        next_action: "deliver_finding_resolution_once",
        artifacts_published: false,
        provider_requests: 0,
        request_id: verified.preflight_request_id,
        egress_request_id: &delivery.request_id,
        review_id: &delivery.review_id,
        candidate_commit: &delivery.candidate_commit,
        session_id,
        route: Route {
            provider: &delivery.provider,
            account: &delivery.account,
            model: &delivery.model,
        },
        prior_packet_hash,
        prior_provider_request_id,
        continuation_anchor_sequence: verified.continuation_anchor_sequence,
        binding_epoch: verified.binding_epoch,
    };
    println!(
        "{}",
        serde_json::to_string(&result).map_err(|error| {
            format!("cannot encode Slice review continuation preflight result: {error}")
        })?
    );
    Ok(())
}

pub(crate) fn evaluate(
    repository: &Path,
    request_path: &Path,
) -> Result<VerifiedContinuation, String> {
    let request_bytes = bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "Slice review continuation preflight request",
    )?;
    let request = parse_request(request_path, &request_bytes)?;
    let egress_request_path = resolve_input_path(repository, &request.egress_request_path);
    let egress_bytes = bounded_file::read_regular(
        &egress_request_path,
        REQUEST_LIMIT,
        "Slice review egress request",
    )?;
    if digest(&egress_bytes) != request.egress_request_hash {
        return Err("Slice review egress request hash does not match its frozen bytes".to_owned());
    }

    let delivery = review_egress::authorize_delivery(repository, &egress_request_path)?;
    let (session_id, prior_packet_hash, prior_provider_request_id) =
        require_finding_resolution(&delivery)?;
    let session_id = session_id
        .parse::<SessionId>()
        .map_err(|error| format!("invalid reviewer Session identity: {error}"))?;
    let session_root = resolve_session_root(repository, &request.session_repository_path)?;
    let reader = LocalSessionReader::open(&session_root)
        .map_err(|error| format!("cannot open reviewer Session repository: {error}"))?;
    let history = read_stored_session(&reader, session_id)
        .map_err(|error| format!("cannot recover reviewer Session: {error}"))?;
    let observation = observe_history(&history, &delivery)?;
    validate_observation(&observation, prior_packet_hash, prior_provider_request_id)?;
    let continuation = read_stored_session_continuation(&reader, session_id).map_err(|error| {
        format!("reviewer Session is not eligible for finding-resolution continuation: {error}")
    })?;
    if continuation.target().session_id() != session_id {
        return Err("recovered continuation target differs from the authorized Session".to_owned());
    }
    let final_history = read_stored_session(&reader, session_id)
        .map_err(|error| format!("cannot revalidate reviewer Session: {error}"))?;
    let final_observation = observe_history(&final_history, &delivery)?;
    if final_observation != observation {
        return Err("reviewer Session changed during continuation preflight".to_owned());
    }
    require_unchanged_file(
        &egress_request_path,
        &egress_bytes,
        "Slice review egress request",
    )?;
    require_unchanged_file(
        request_path,
        &request_bytes,
        "Slice review continuation preflight request",
    )?;

    Ok(VerifiedContinuation {
        preflight_request_id: digest(&request_bytes),
        delivery,
        session_root,
        continuation_anchor_sequence: continuation
            .target()
            .source_anchor_sequence()
            .ok_or_else(|| {
                "Slice review continuation requires a Continuation Anchor source".to_owned()
            })?
            .get(),
        binding_epoch: continuation.target().epoch(),
    })
}

fn parse_request(path: &Path, bytes: &[u8]) -> Result<Request, String> {
    let request: Request = serde_json::from_slice(bytes).map_err(|error| {
        format!(
            "invalid Slice review continuation preflight request {}: {error}",
            path.display()
        )
    })?;
    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "unsupported Slice review continuation preflight request schema `{}`; expected `{REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    compact_path(&request.egress_request_path, "egress_request_path")?;
    compact_path(&request.session_repository_path, "session_repository_path")?;
    require_sha256(&request.egress_request_hash, "egress_request_hash")?;
    Ok(request)
}

fn require_finding_resolution(delivery: &AuthorizedDelivery) -> Result<(&str, &str, &str), String> {
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

fn resolve_session_root(repository: &Path, requested: &str) -> Result<PathBuf, String> {
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

fn observe_history(
    history: &yo_core::session_repository::StoredSessionHistory,
    delivery: &AuthorizedDelivery,
) -> Result<Observation, String> {
    let start_packet_hashes = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::CommandCommitted(AgentCommand::StartTurn { input, .. }) => {
                Some(digest(input.as_str().as_bytes()))
            },
            _ => None,
        })
        .collect();
    let mut binding_matches = Vec::new();
    let mut request_identities = Vec::new();
    let mut outcome_identities = Vec::new();
    let mut continuation_anchors = Vec::new();
    for entry in history.request_trace() {
        match entry.record() {
            StoredRequestTraceRecord::BindingOpened {
                binding_identity, ..
            } => binding_matches.push(managed_binding_matches(
                binding_identity.value(),
                &delivery.provider,
                &delivery.account,
                &delivery.model,
            )?),
            StoredRequestTraceRecord::RequestAccepted {
                request_identity, ..
            } => request_identities.push(request_identity.value().to_owned()),
            StoredRequestTraceRecord::ResumableOutcome {
                outcome_identity, ..
            } => outcome_identities.push(
                outcome_identity
                    .as_ref()
                    .map(|identity| identity.value().to_owned()),
            ),
            StoredRequestTraceRecord::ContinuationAnchor {
                epoch,
                accepted_request_sequence,
                resumable_outcome_sequence,
                journal_boundary,
            } => continuation_anchors.push((
                *epoch,
                accepted_request_sequence.get(),
                resumable_outcome_sequence.get(),
                journal_boundary.get(),
            )),
            _ => {},
        }
    }
    Ok(Observation {
        start_packet_hashes,
        binding_matches,
        request_identities,
        outcome_identities,
        continuation_anchors,
    })
}

fn validate_observation(
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
    let observed = provider_request_identity(
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

fn require_unchanged_file(path: &Path, expected: &[u8], label: &str) -> Result<(), String> {
    let actual = bounded_file::read_regular(path, REQUEST_LIMIT, label)?;
    if actual != expected {
        return Err(format!("{label} changed during continuation preflight"));
    }
    Ok(())
}

fn compact_path(value: &str, name: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_PATH_BYTES || value.contains('\0') {
        return Err(format!("{name} must be a non-empty bounded path"));
    }
    Ok(())
}

fn require_sha256(value: &str, name: &str) -> Result<(), String> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(format!("{name} must use sha256:<hex>"));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{name} must use sha256:<64 lowercase hex>"));
    }
    if hex.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(format!("{name} must use lowercase hexadecimal"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // 새 preflight wire는 v1alpha1의 closed shape만 받아 stable 추측이나 caller가 넣은
    // effect option이 read-only 검사로 해석되지 않게 합니다.
    #[test]
    fn request_requires_the_exact_v1alpha1_shape() {
        let valid = r#"{
            "schema":"yo.slice-review-continuation-preflight-request/v1alpha1",
            "egress_request_path":"egress.json",
            "egress_request_hash":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "session_repository_path":"sessions"
        }"#;
        parse_request(Path::new("request.json"), valid.as_bytes()).unwrap();

        let stable = valid.replace("request/v1alpha1", "request/v1");
        assert!(
            parse_request(Path::new("request.json"), stable.as_bytes())
                .unwrap_err()
                .contains("v1alpha1")
        );
        let extra = valid.replace("\n        }", ",\n            \"retry\":1\n        }");
        assert!(
            parse_request(Path::new("request.json"), extra.as_bytes())
                .unwrap_err()
                .contains("unknown field")
        );
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
