use crate::review_protocol::digest;

pub(super) const START: &[u8] = b"<<<YO-SLICE-REVIEW-RESULT>>>";
pub(super) const END: &[u8] = b"<<<YO-SLICE-REVIEW-RESULT-END>>>";
pub(super) const MAX_FINDINGS: usize = 64;
pub(super) const MAX_SUMMARY_BYTES: usize = 4096;

pub(super) fn require_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(format!(
            "{label} must be a canonical sha256:<64 lowercase hex> identity"
        ))
    }
}

pub(super) fn require_exact_hash(bytes: &[u8], expected: &str, label: &str) -> Result<(), String> {
    if digest(bytes) == expected {
        Ok(())
    } else {
        Err(format!("{label} hash does not match its frozen bytes"))
    }
}

pub(super) fn exactly_one(bytes: &[u8], needle: &[u8], label: &str) -> Result<usize, String> {
    let mut matches = bytes
        .windows(needle.len())
        .enumerate()
        .filter_map(|(index, value)| (value == needle).then_some(index));
    let first = matches
        .next()
        .ok_or_else(|| format!("review output is missing {label}"))?;
    if matches.next().is_some() {
        return Err(format!("review output contains more than one {label}"));
    }
    Ok(first)
}

pub(super) fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

pub(super) fn compact(value: &str, limit: usize, label: &str) -> Result<(), String> {
    if value.is_empty() || value != value.trim() || value.len() > limit || value.contains('\0') {
        Err(format!(
            "structured review {label} must be non-empty, trimmed, and at most {limit} bytes"
        ))
    } else {
        Ok(())
    }
}

pub(super) fn compact_token(value: &str, limit: usize, label: &str) -> Result<(), String> {
    compact(value, limit, label)?;
    if value
        .bytes()
        .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        Err(format!(
            "structured review {label} must be one compact token"
        ))
    } else {
        Ok(())
    }
}
