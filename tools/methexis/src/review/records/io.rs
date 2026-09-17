//! Review record의 bounded 읽기와 텍스트 입력 검증을 담당한다.

use std::{fs, io::Read, path::Path, str};

use crate::{
    check::Diagnostic,
    review::{MAX_RECORD_BYTES, local_diagnostic, relative_path},
};

#[allow(clippy::result_large_err)]
pub(super) fn decode_utf8<'a>(
    bytes: &'a [u8],
    path: &Path,
    repository_root: &Path,
    code: &str,
) -> Result<&'a str, Diagnostic> {
    str::from_utf8(bytes).map_err(|error| {
        local_diagnostic(
            relative_path(repository_root, path),
            code,
            error.to_string(),
            Vec::new(),
        )
    })
}

#[allow(clippy::result_large_err)]
pub(super) fn read_record(
    path: &Path,
    repository_root: &Path,
    unreadable_code: &str,
    record_name: &str,
) -> Result<Vec<u8>, Diagnostic> {
    let display = relative_path(repository_root, path);
    let mut file = fs::File::open(path).map_err(|error| {
        local_diagnostic(
            display.clone(),
            unreadable_code,
            error.to_string(),
            Vec::new(),
        )
    })?;
    let mut bytes = Vec::new();
    Read::take(&mut file, (MAX_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            local_diagnostic(
                display.clone(),
                unreadable_code,
                error.to_string(),
                Vec::new(),
            )
        })?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(local_diagnostic(
            display,
            "review_record_too_large",
            format!("{record_name} exceeds {MAX_RECORD_BYTES} bytes"),
            Vec::new(),
        ));
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(local_diagnostic(
            display,
            "review_record_bom_forbidden",
            format!("{record_name} must not start with a UTF-8 BOM"),
            Vec::new(),
        ));
    }
    Ok(bytes)
}
