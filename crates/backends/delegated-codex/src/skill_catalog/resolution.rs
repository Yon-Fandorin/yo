//! bounded skill instruction snapshot, digest, resolution을 담당합니다.

use std::{fmt::Write as _, fs::OpenOptions, io::Read as _, os::unix::fs::OpenOptionsExt as _};

use sha2::{Digest, Sha256};
use yo_core::{ResolvedSkill, SkillReference, SubmissionRejection, SubmissionRejectionKind};

pub(super) fn resolve_selected_skill(
    selected: SkillReference,
) -> Result<ResolvedSkill, SubmissionRejection> {
    // authoritative catalog을 먼저 확인했으므로 같은 bounded bytes로 digest와 model input을
    // 만들며, 이후에 경로를 다시 조회하지 않습니다.
    let instructions = skill_instructions(selected.locator())?;
    if skill_revision(&instructions) != selected.entry_revision() {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::StaleReference,
            "selected skill contents changed; select it again",
        ));
    }
    ResolvedSkill::new(selected, instructions).map_err(|error| {
        SubmissionRejection::new(
            SubmissionRejectionKind::RequiredAssetUnavailable,
            error.to_string(),
        )
    })
}

pub(super) fn skill_digest(path: &str) -> Result<String, String> {
    skill_instructions(path)
        .map(|text| skill_revision(&text))
        .map_err(|error| error.message().to_owned())
}

fn skill_instructions(path: &str) -> Result<String, SubmissionRejection> {
    let unavailable = |message: String| {
        SubmissionRejection::new(SubmissionRejectionKind::RequiredAssetUnavailable, message)
    };
    // 같은 descriptor를 확인하기 전에 nonblocking으로 열어 FIFO writer를 기다리지 않습니다.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| unavailable(format!("Skill revision unavailable: {error}")))?;
    let metadata = file
        .metadata()
        .map_err(|error| unavailable(format!("Skill revision unavailable: {error}")))?;
    if !metadata.is_file() {
        return Err(unavailable(
            "Skill revision unavailable: not a regular file".to_owned(),
        ));
    }
    let limit = ResolvedSkill::MAX_INSTRUCTION_BYTES;
    if metadata.len() > limit as u64 {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::OverBudget,
            "Skill revision unavailable: instruction byte limit exceeded",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| unavailable(format!("Skill revision unavailable: {error}")))?;
    if bytes.len() > limit {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::OverBudget,
            "Skill revision unavailable: instruction byte limit exceeded",
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        unavailable("Skill revision unavailable: instructions are not valid UTF-8".to_owned())
    })
}

pub(super) fn skill_revision(instructions: &str) -> String {
    let mut revision = String::from("sha256:");
    for byte in Sha256::digest(instructions.as_bytes()) {
        let _ = write!(revision, "{byte:02x}");
    }
    revision
}
