use std::path::Path;

use serde::Deserialize;

use super::{MAX_PATH_BYTES, REQUEST_SCHEMA};

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

pub(super) fn compact_path(value: &str, name: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_PATH_BYTES || value.contains('\0') {
        return Err(format!("{name} must be a non-empty bounded path"));
    }
    Ok(())
}

pub(super) fn require_sha256(value: &str, name: &str) -> Result<(), String> {
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
    use std::path::Path;

    use super::parse_request;

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
}
