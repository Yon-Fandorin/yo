//! Review operation request의 bounded decode와 입력 정규화를 담당한다.

use std::{fs, io::Read, path::Path};

use serde::de;

use crate::{
    check,
    review::{MAX_REQUEST_BYTES, OperationFailure},
};

pub(crate) fn read_request<T>(path: &Path, operation: &'static str) -> Result<T, OperationFailure>
where
    T: de::DeserializeOwned,
{
    let mut file = fs::File::open(path).map_err(|error| {
        OperationFailure::new(
            operation,
            "request_unreadable",
            error.to_string(),
            Vec::new(),
            "provide a readable versioned JSON request file",
        )
    })?;
    let mut bytes = Vec::new();
    Read::take(&mut file, (MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            OperationFailure::new(
                operation,
                "request_unreadable",
                error.to_string(),
                Vec::new(),
                "provide a readable versioned JSON request file",
            )
        })?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(OperationFailure::new(
            operation,
            "request_too_large",
            format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
            Vec::new(),
            "reduce the request to the required fields",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        OperationFailure::new(
            operation,
            "invalid_request",
            error.to_string(),
            Vec::new(),
            "repair the versioned JSON request",
        )
    })
}

pub(crate) fn require_schema(
    operation: &'static str,
    actual: &str,
    expected: &str,
    id: &str,
) -> Result<(), OperationFailure> {
    if actual == expected {
        Ok(())
    } else {
        Err(OperationFailure::new(
            operation,
            "unsupported_request_schema",
            format!("expected request schema `{expected}`"),
            vec![id.to_owned()],
            "regenerate the request using the current schema",
        ))
    }
}

pub(crate) fn normalize_markdown(
    markdown: &str,
    operation: &'static str,
    id: &str,
) -> Result<String, OperationFailure> {
    let normalized = markdown.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalized.trim().to_owned();
    if normalized.is_empty() {
        return Err(OperationFailure::new(
            operation,
            "empty_korean_markdown",
            "Korean review Markdown must not be empty",
            vec![id.to_owned()],
            "provide the exact Korean text to preserve for review",
        ));
    }
    if check::body_has_forbidden_html(&normalized) {
        return Err(OperationFailure::new(
            operation,
            "raw_html_forbidden",
            "Korean review Markdown must not contain raw HTML blocks or comments",
            vec![id.to_owned()],
            "use visible Markdown or fenced code",
        ));
    }
    Ok(normalized)
}
