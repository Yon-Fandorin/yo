use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::super::{
    REQUEST_LIMIT,
    artifact::{compact_path, require_exact_file_hash, require_sha256},
    workspace::shared_path,
};
use crate::{bounded_file, review_egress};

const REQUEST_SCHEMA: &str = "yo.slice-review-delegated-delivery-finalize-request/v1alpha1";
const CONTINUATION_PREFLIGHT_SCHEMA: &str =
    "yo.slice-review-delegated-continuation-preflight-request/v1alpha1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) schema: String,
    pub(super) delivery_request_path: String,
    pub(super) delivery_request_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ContinuationPreflightRequest {
    pub(super) schema: String,
    pub(super) egress_request_path: String,
    pub(super) egress_request_hash: String,
    pub(super) session_repository_path: String,
}

pub(super) struct LoadedRequest {
    pub(super) request: Request,
    pub(super) bytes: Vec<u8>,
    pub(super) delivery_request_path: PathBuf,
}

pub(super) fn read(repository: &Path, request_path: &Path) -> Result<LoadedRequest, String> {
    let request_bytes = bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "delegated delivery finalization request",
    )?;
    let request: Request = serde_json::from_slice(&request_bytes).map_err(|error| {
        format!(
            "invalid delegated delivery finalization request {}: {error}",
            request_path.display()
        )
    })?;
    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "unsupported delegated delivery finalization schema `{}`; expected `{REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    compact_path(&request.delivery_request_path, "delivery_request_path")?;
    require_sha256(&request.delivery_request_hash, "delivery_request_hash")?;
    let delivery_request_path = shared_path(repository, &request.delivery_request_path)?;
    require_exact_file_hash(
        &delivery_request_path,
        &request.delivery_request_hash,
        REQUEST_LIMIT,
        "delegated delivery request",
    )?;
    Ok(LoadedRequest {
        request,
        bytes: request_bytes,
        delivery_request_path,
    })
}

pub(super) fn read_continuation_preflight(
    bytes: &[u8],
) -> Result<ContinuationPreflightRequest, String> {
    let request: ContinuationPreflightRequest = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid delegated continuation preflight request: {error}"))?;
    if request.schema != CONTINUATION_PREFLIGHT_SCHEMA {
        return Err("unsupported delegated continuation preflight schema".to_owned());
    }
    Ok(request)
}

pub(super) fn validate_claim(
    claim: &serde_json::Value,
    delivery: &review_egress::AuthorizedHostDelivery,
    admission_hash: &str,
    expected_schema: &str,
    continuation: bool,
    preflight_id: Option<&str>,
    execution_isolation: Option<&str>,
) -> Result<(), String> {
    require_claim_schema(claim, expected_schema)?;
    for (field, expected) in [
        ("request_id", delivery.request_id.as_str()),
        ("authorization_id", delivery.authorization_id.as_str()),
        ("authority", delivery.authority.as_str()),
        ("review_id", delivery.review_id.as_str()),
        ("candidate_commit", delivery.candidate_commit.as_str()),
        ("integration_commit", delivery.trusted_commit.as_str()),
        ("packet_hash", delivery.packet_hash.as_str()),
        ("execution_profile", delivery.execution_profile.as_str()),
        ("admission_request_id", admission_hash),
    ] {
        if claim.get(field).and_then(serde_json::Value::as_str) != Some(expected) {
            return Err(format!("delegated delivery claim changed `{field}`"));
        }
    }
    if claim
        .get("execution_isolation")
        .and_then(serde_json::Value::as_str)
        != execution_isolation
    {
        return Err("delegated delivery claim changed `execution_isolation`".to_owned());
    }
    if claim
        .pointer("/target/kind")
        .and_then(serde_json::Value::as_str)
        != Some("delegated_host")
        || claim
            .pointer("/target/host")
            .and_then(serde_json::Value::as_str)
            != Some(delivery.host.as_str())
        || claim
            .get("packet_bytes")
            .and_then(serde_json::Value::as_u64)
            != Some(delivery.packet_bytes.len() as u64)
        || claim
            .get("host_request_limit")
            .and_then(serde_json::Value::as_u64)
            != Some(1)
        || ["retries", "steer", "fallback"]
            .iter()
            .any(|field| claim.get(*field).and_then(serde_json::Value::as_u64) != Some(0))
        || claim
            .get("target_switch")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
    {
        return Err("delegated delivery claim limits or identity changed".to_owned());
    }
    let expected_mode = if continuation { "resume" } else { "fresh" };
    if claim
        .get("session_mode")
        .and_then(serde_json::Value::as_str)
        != Some(expected_mode)
    {
        return Err("delegated delivery claim changed Session mode".to_owned());
    }
    if continuation
        && (claim
            .get("preflight_request_id")
            .and_then(serde_json::Value::as_str)
            != preflight_id
            || claim.get("session_id").and_then(serde_json::Value::as_str)
                != delivery.session_id.as_deref()
            || claim
                .get("prior_host_request_id")
                .and_then(serde_json::Value::as_str)
                != delivery.prior_host_request_id.as_deref())
    {
        return Err("delegated continuation claim changed its prior boundary".to_owned());
    }
    Ok(())
}

pub(super) fn require_claim_schema(
    claim: &serde_json::Value,
    expected: &str,
) -> Result<(), String> {
    if claim.get("schema").and_then(serde_json::Value::as_str) == Some(expected) {
        Ok(())
    } else {
        Err(format!(
            "delegated delivery claim schema must equal `{expected}`"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::super::model::{DELEGATED_CLAIM_SCHEMA, DELEGATED_CLAIM_SCHEMA_V1_ALPHA2},
        REQUEST_SCHEMA, Request, require_claim_schema,
    };

    // 복구 요청은 전송 입력을 exact hash로만 가리키며 실행 파일·재시도 옵션을
    // 표현할 수 없어 provider/host request를 추가하는 입력 공간이 없습니다.
    #[test]
    fn finalization_request_has_no_delivery_effect_fields() {
        let valid = serde_json::json!({
            "schema": REQUEST_SCHEMA,
            "delivery_request_path": "/tmp/delivery-request.json",
            "delivery_request_hash": format!("sha256:{}", "a".repeat(64))
        });
        serde_json::from_value::<Request>(valid.clone()).unwrap();
        let mut extra = valid;
        extra["retry"] = true.into();
        assert!(serde_json::from_value::<Request>(extra).is_err());
    }

    // 복구는 필드가 우연히 같은 미지의 claim을 수용하지 않고 delivery request가
    // 소유하는 frozen claim schema를 exact wire boundary로 먼저 확인합니다.
    #[test]
    fn finalization_requires_the_exact_claim_schema() {
        let exact = serde_json::json!({"schema": DELEGATED_CLAIM_SCHEMA_V1_ALPHA2});
        require_claim_schema(&exact, DELEGATED_CLAIM_SCHEMA_V1_ALPHA2).unwrap();
        assert!(
            require_claim_schema(&exact, DELEGATED_CLAIM_SCHEMA)
                .unwrap_err()
                .contains(DELEGATED_CLAIM_SCHEMA)
        );
        let unknown = serde_json::json!({"schema": "yo.unknown-claim/v1alpha1"});
        assert!(require_claim_schema(&unknown, DELEGATED_CLAIM_SCHEMA).is_err());
    }
}
