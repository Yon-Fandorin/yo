use std::{ffi::OsStr, fmt::Write as _, fs::File, io::Read};

use rustix::fs::{Mode, OFlags, openat};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    super::SkillAvailability,
    roots::{DIRECTORY_FLAGS, Root},
};
use crate::{ResolvedSkill, SubmissionRejection, SubmissionRejectionKind};

#[derive(Default, Deserialize)]
struct Metadata {
    name: Option<String>,
    description: Option<String>,
    enabled: Option<bool>,
    #[serde(rename = "user-invocable")]
    user_invocable: Option<bool>,
}

pub(super) struct Snapshot {
    pub(super) text: String,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) revision: String,
    pub(super) availability: SkillAvailability,
}

pub(super) fn read_snapshot(
    root: &Root,
    child: &str,
    budget: usize,
) -> Result<Snapshot, SubmissionRejection> {
    let unavailable =
        |error: String| super::reject(SubmissionRejectionKind::RequiredAssetUnavailable, error);
    let parent = root.open().map_err(unavailable)?;
    let directory =
        openat(parent, OsStr::new(child), DIRECTORY_FLAGS, Mode::empty()).map_err(|error| {
            unavailable(format!(
                "skill directory unavailable (symlinks are not supported): {error}"
            ))
        })?;
    let descriptor = openat(
        directory,
        "SKILL.md",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        unavailable(format!(
            "SKILL.md unavailable (symlinks are not supported): {error}"
        ))
    })?;
    let file = File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|error| unavailable(error.to_string()))?;
    if !metadata.is_file() {
        return Err(unavailable("SKILL.md is not a regular file".into()));
    }
    let limit = ResolvedSkill::MAX_INSTRUCTION_BYTES.min(budget);
    if metadata.len() > limit as u64 {
        return Err(super::reject(
            SubmissionRejectionKind::OverBudget,
            "skill instruction or scan byte limit exceeded",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| unavailable(error.to_string()))?;
    if bytes.len() > limit {
        return Err(super::reject(
            SubmissionRejectionKind::OverBudget,
            "skill instruction or scan byte limit exceeded",
        ));
    }
    root.open()
        .map_err(|error| super::reject(SubmissionRejectionKind::EnvironmentUnavailable, error))?;
    let text = String::from_utf8(bytes).map_err(|_| unavailable("SKILL.md is not UTF-8".into()))?;
    let metadata = if text.starts_with("---\n") || text.starts_with("---\r\n") {
        let start = text.find('\n').expect("frontmatter opening has newline") + 1;
        let mut offset = start;
        let mut end = None;
        for line in text[start..].split_inclusive('\n') {
            if line.trim_end_matches(['\r', '\n']) == "---" {
                end = Some(offset);
                break;
            }
            offset += line.len();
        }
        let header =
            &text[start..end.ok_or_else(|| unavailable("unclosed skill frontmatter".into()))?];
        if header.len() > 16 * 1024 {
            return Err(unavailable("skill frontmatter exceeds 16 KiB".into()));
        }
        yo_yaml::from_str_with_limits::<Metadata>(
            header,
            yo_yaml::ParseLimits::with_max_total_scalar_bytes(16 * 1024),
        )
        .map_err(|error| unavailable(format!("invalid skill frontmatter: {error}")))?
    } else {
        Metadata::default()
    };
    let name = metadata.name.unwrap_or_else(|| child.to_owned());
    let description = metadata.description.unwrap_or_default();
    if name.is_empty()
        || name.len() > 128
        || name.chars().any(|ch| ch.is_whitespace() || ch.is_control())
        || description.len() > 4096
        || description
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\r' | '\n'))
    {
        return Err(unavailable("invalid skill name or description".into()));
    }
    let revision = format!("sha256:{}", digest(text.as_bytes()));
    let availability = if !metadata.enabled.unwrap_or(true) {
        SkillAvailability::Disabled("disabled by local skill metadata".into())
    } else if !metadata.user_invocable.unwrap_or(true) {
        SkillAvailability::Disabled("local skill is not user-invocable".into())
    } else if text.trim().is_empty() {
        SkillAvailability::Disabled("skill instructions are blank".into())
    } else {
        SkillAvailability::Enabled
    };
    Ok(Snapshot {
        text,
        name,
        description,
        revision,
        availability,
    })
}

pub(super) fn digest(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}
