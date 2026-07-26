//! Exact direct-anchor resolution in the pinned trusted KnowledgeSnapshot.

use super::ResolvedAnchor;
use crate::{
    checkpoint::ContextAuthority,
    context::wire::{Anchor, ResolveFailure},
};

pub(super) fn resolve(
    authority: &ContextAuthority,
    anchors: &[Anchor],
) -> Result<Vec<ResolvedAnchor>, ResolveFailure> {
    let mut resolved = Vec::with_capacity(anchors.len());
    for anchor in anchors {
        let value = anchor.value().trim();
        let mut roots = authority
            .foundation
            .units
            .iter()
            .filter(|unit| matches_unit(authority, unit, anchor, value))
            .map(|unit| unit.metadata.id.clone())
            .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        if roots.is_empty() {
            return Err(ResolveFailure::new(
                Some(authority.trusted_commit.clone()),
                "explicit_anchor_unresolved",
                format!(
                    "{} anchor `{value}` resolved to no trusted KnowledgeUnit",
                    anchor.kind()
                ),
                false,
                Vec::new(),
                Vec::new(),
                "correct the explicit anchor or activate matching knowledge",
            ));
        }
        resolved.push(ResolvedAnchor {
            anchor: trimmed(anchor),
            roots,
        });
    }
    Ok(resolved)
}

fn matches_unit(
    authority: &ContextAuthority,
    unit: &crate::model::KnowledgeUnit,
    anchor: &Anchor,
    value: &str,
) -> bool {
    match anchor {
        Anchor::KnowledgeId { .. } => unit.metadata.id == value,
        Anchor::Path { .. } => {
            authority
                .unit_paths
                .get(&unit.metadata.id)
                .map(String::as_str)
                == Some(value)
                || applies_to(unit, value)
        },
        Anchor::Symbol { .. } => applies_to(unit, value),
    }
}

fn applies_to(unit: &crate::model::KnowledgeUnit, value: &str) -> bool {
    unit.metadata
        .relations
        .applies_to
        .iter()
        .any(|target| target == value)
}

fn trimmed(anchor: &Anchor) -> Anchor {
    let value = anchor.value().trim().to_owned();
    match anchor {
        Anchor::KnowledgeId { .. } => Anchor::KnowledgeId { value },
        Anchor::Path { .. } => Anchor::Path { value },
        Anchor::Symbol { .. } => Anchor::Symbol { value },
    }
}
