//! Codex `skills/list` 경계의 공개 facade와 비공개 구현 모듈입니다.

mod admission;
mod catalog;
mod resolution;
mod worker;

#[cfg(test)]
mod tests;

pub use admission::CodexSkillInputAdmission;
#[cfg(test)]
use admission::validate_selected_skill;
#[cfg(test)]
use catalog::{SkillInterface, SkillMetadata, WireScope, candidate_from_wire};
#[cfg(test)]
use resolution::{resolve_selected_skill, skill_digest, skill_revision};
pub use worker::CodexSkillReferenceProvider;
#[cfg(test)]
use worker::newest_request;
