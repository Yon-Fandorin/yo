use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use self::storage::{CapturedFile, RepositoryFiles};

mod storage;

#[cfg(test)]
mod tests;

const CANONICAL_ROOT: &str = "docs/src";
const KOREAN_ROOT: &str = "docs/ko/src";
const MANIFEST: &str = "docs/ko/source.sha256";
const MAX_PAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

pub(crate) fn accept(repository: &Path, page: &Path) -> Result<(), String> {
    let page = validated_page(page)?;
    let files = RepositoryFiles::open(repository)?;
    let canonical = files.capture(&Path::new(CANONICAL_ROOT).join(&page), MAX_PAGE_BYTES)?;
    let korean = files.capture(&Path::new(KOREAN_ROOT).join(&page), MAX_PAGE_BYTES)?;
    let manifest = files.capture(Path::new(MANIFEST), MAX_MANIFEST_BYTES)?;

    let digest = sha256_hex(canonical.bytes());
    let page = page
        .to_str()
        .ok_or_else(|| "Developer Docs page path must be UTF-8".to_owned())?;
    let updated = update_manifest(manifest.bytes(), page, &digest)?;
    if updated != manifest.bytes() {
        publish_reviewed_hash(&canonical, &korean, &manifest, &updated)?;
    }
    println!("accepted translation: {page} sha256:{digest}");
    Ok(())
}

fn publish_reviewed_hash(
    canonical: &CapturedFile,
    korean: &CapturedFile,
    manifest: &CapturedFile,
    updated: &[u8],
) -> Result<(), String> {
    manifest.atomic_replace_guarded(updated, || {
        canonical.revalidate()?;
        korean.revalidate()
    })
}

fn validated_page(page: &Path) -> Result<PathBuf, String> {
    if page.as_os_str().is_empty()
        || page.extension().and_then(|value| value.to_str()) != Some("md")
    {
        return Err("translation page must be a relative .md path below docs/src".to_owned());
    }
    if page
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("translation page must not be absolute or contain `.` or `..`".to_owned());
    }
    let page_text = page
        .to_str()
        .ok_or_else(|| "Developer Docs page path must be UTF-8".to_owned())?;
    if page_text.contains(['\\', '\n', '\r']) {
        return Err(
            "translation page contains characters unsupported by the hash manifest".to_owned(),
        );
    }
    Ok(page.components().collect())
}

fn update_manifest(manifest: &[u8], page: &str, digest: &str) -> Result<Vec<u8>, String> {
    let manifest = std::str::from_utf8(manifest)
        .map_err(|_| "docs/ko/source.sha256 must be UTF-8".to_owned())?;
    let mut updated = String::with_capacity(manifest.len());
    let mut seen = HashSet::new();
    let mut selected = false;
    for raw_line in manifest.split_inclusive('\n') {
        let (content, newline) = raw_line
            .strip_suffix('\n')
            .map_or((raw_line, ""), |line| (line, "\n"));
        let (content, carriage) = content
            .strip_suffix('\r')
            .map_or((content, ""), |line| (line, "\r"));
        let bytes = content.as_bytes();
        if bytes.len() < 67
            || !bytes[..64]
                .iter()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            || &bytes[64..66] != b"  "
        {
            return Err("docs/ko/source.sha256 contains a malformed entry".to_owned());
        }
        let entry = &content[66..];
        let normalized = validated_page(Path::new(entry))?;
        if normalized.to_str() != Some(entry) {
            return Err(format!("manifest path is not normalized: {entry}"));
        }
        if !seen.insert(entry) {
            return Err(format!(
                "{entry} has duplicate entries in docs/ko/source.sha256"
            ));
        }
        if entry == page {
            selected = true;
            updated.push_str(digest);
            updated.push_str(&content[64..]);
        } else {
            updated.push_str(content);
        }
        updated.push_str(carriage);
        updated.push_str(newline);
    }
    if !selected {
        return Err(format!("{page} has no entry in docs/ko/source.sha256"));
    }
    Ok(updated.into_bytes())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
