use std::path::Path;

use serde::Deserialize;

use super::{
    super::request::{compact_path, require_sha256},
    REQUEST_SCHEMA,
};
use crate::review_egress::AuthorizedHostDelivery;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) schema: String,
    pub(super) egress_request_path: String,
    pub(super) egress_request_hash: String,
    pub(super) session_repository_path: String,
}

pub(super) fn parse_request(path: &Path, bytes: &[u8]) -> Result<Request, String> {
    let request: Request = serde_json::from_slice(bytes).map_err(|error| {
        format!(
            "invalid delegated continuation preflight request {}: {error}",
            path.display()
        )
    })?;
    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "unsupported delegated continuation preflight schema `{}`; expected `{REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    compact_path(&request.egress_request_path, "egress_request_path")?;
    compact_path(&request.session_repository_path, "session_repository_path")?;
    require_sha256(&request.egress_request_hash, "egress_request_hash")?;
    Ok(request)
}

pub(super) fn require_finding_resolution(
    delivery: &AuthorizedHostDelivery,
) -> Result<(&str, &str, &str), String> {
    if delivery.review_kind != "finding_resolution" || delivery.fresh_session {
        return Err(
            "delegated continuation preflight accepts only an authorized finding-resolution resume"
                .to_owned(),
        );
    }
    let session_id = delivery
        .session_id
        .as_deref()
        .ok_or_else(|| "delegated finding-resolution has no reviewer Session".to_owned())?;
    let packet_hash = delivery
        .prior_packet_hash
        .as_deref()
        .ok_or_else(|| "delegated finding-resolution has no prior packet hash".to_owned())?;
    let request_id = delivery.prior_host_request_id.as_deref().ok_or_else(|| {
        "delegated finding-resolution has no prior host request identity".to_owned()
    })?;
    Ok((session_id, packet_hash, request_id))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{REQUEST_SCHEMA, parse_request};
    use crate::review_continuation_preflight::REQUEST_SCHEMA as MANAGED_REQUEST_SCHEMA;

    // delegated continuation preflight는 managed schema나 Provider 좌표를 받아들이지 않고
    // exact egress bytes와 Session repository만 가리키는 closed alpha shape를 유지합니다.
    #[test]
    fn request_has_a_closed_delegated_alpha_shape() {
        let value = serde_json::json!({
            "schema": REQUEST_SCHEMA,
            "egress_request_path": ".local-exclude/egress.json",
            "egress_request_hash": format!("sha256:{}", "a".repeat(64)),
            "session_repository_path": "/tmp/sessions"
        });
        parse_request(
            Path::new("request.json"),
            &serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();

        let managed = serde_json::json!({
            "schema": MANAGED_REQUEST_SCHEMA,
            "egress_request_path": ".local-exclude/egress.json",
            "egress_request_hash": format!("sha256:{}", "a".repeat(64)),
            "session_repository_path": "/tmp/sessions"
        });
        assert!(
            parse_request(
                Path::new("request.json"),
                &serde_json::to_vec(&managed).unwrap()
            )
            .unwrap_err()
            .contains("unsupported delegated continuation preflight schema")
        );

        let mut fabricated = value;
        fabricated["provider"] = "codex".into();
        assert!(
            parse_request(
                Path::new("request.json"),
                &serde_json::to_vec(&fabricated).unwrap()
            )
            .unwrap_err()
            .contains("unknown field")
        );
    }
}
