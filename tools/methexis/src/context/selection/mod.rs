//! Trusted anchor resolution, atomic closure expansion, and greedy packing.

mod anchors;
mod graph;
mod state;

use std::collections::BTreeSet;

use serde::Serialize;

use super::wire::{Anchor, Candidate, ResolveFailure};
use crate::{checkpoint::ContextAuthority, model::KnowledgeUnit};

#[derive(Clone, Debug, Serialize)]
pub(super) struct ResolvedAnchor {
    pub(super) anchor: Anchor,
    pub(super) roots: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct CandidateDecision {
    pub(super) candidate: Candidate,
    pub(super) disposition: &'static str,
    pub(super) reason: String,
    pub(super) bundle: Vec<String>,
    pub(super) marginal_tokens: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct UnitObservation {
    pub(super) id: String,
    pub(super) eligibility: &'static str,
    pub(super) evidence: Vec<String>,
    pub(super) approval: Option<ApprovalObservation>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ApprovalObservation {
    pub(super) projection_hash: String,
    pub(super) projection_profile: String,
    pub(super) projection_compiler: String,
    pub(super) approval_hash: String,
}

pub(super) struct Selection {
    pub(super) required_roots: Vec<String>,
    pub(super) anchors: Vec<ResolvedAnchor>,
    pub(super) included: BTreeSet<String>,
    pub(super) decisions: Vec<CandidateDecision>,
    pub(super) observations: Vec<UnitObservation>,
    pub(super) token_count: usize,
}

pub(super) fn ordered_units<'a>(
    included: &BTreeSet<String>,
    units: &'a [KnowledgeUnit],
) -> Vec<&'a KnowledgeUnit> {
    graph::ordered_units(included, units)
}

pub(super) fn pack(
    authority: &ContextAuthority,
    anchors: &[Anchor],
    candidates: &[Candidate],
    budget: usize,
    mut measure: impl FnMut(&BTreeSet<String>) -> usize,
) -> Result<Selection, ResolveFailure> {
    let units = graph::unit_map(&authority.foundation.units);
    let resolved_anchors = anchors::resolve(authority, anchors)?;
    let required_roots = resolved_anchors
        .iter()
        .flat_map(|resolved| resolved.roots.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut observed = BTreeSet::new();
    let mut included = BTreeSet::new();
    for root in &required_roots {
        let bundle = graph::closure(root, &units).map_err(|missing| {
            failure(
                authority,
                "required_knowledge_missing",
                format!("required KnowledgeId `{missing}` is missing"),
                vec![root.clone(), missing],
                "repair and reactivate the required knowledge closure",
            )
        })?;
        observed.extend(bundle.iter().cloned());
        let blocked = state::blocked_units(authority, &bundle);
        if !blocked.is_empty() {
            return Err(failure(
                authority,
                "required_knowledge_blocked",
                format!("required bundle for `{root}` is not fully approved and active"),
                blocked,
                "restore the required knowledge and active Checkpoint before retrying",
            ));
        }
        included.extend(bundle);
    }
    let required_tokens = measure(&included);
    if required_tokens > budget {
        return Err(failure(
            authority,
            "required_budget_exceeded",
            format!("required knowledge needs {required_tokens} tokens but the budget is {budget}"),
            required_roots.iter().cloned().collect(),
            "increase max_tokens or reduce the explicit required roots",
        ));
    }

    let mut decisions = Vec::with_capacity(candidates.len());
    let mut token_count = required_tokens;
    for candidate in candidates {
        let Some(_) = units.get(candidate.id.as_str()) else {
            decisions.push(decision(
                candidate,
                "omitted",
                "unknown_knowledge_id",
                BTreeSet::new(),
                0,
            ));
            continue;
        };
        let bundle = graph::closure(&candidate.id, &units).map_err(|missing| {
            failure(
                authority,
                "trusted_graph_invalid",
                format!("candidate closure references missing KnowledgeId `{missing}`"),
                vec![candidate.id.clone(), missing],
                "repair the trusted knowledge graph",
            )
        })?;
        observed.extend(bundle.iter().cloned());
        let blocked = state::blocked_units(authority, &bundle);
        if !blocked.is_empty() {
            decisions.push(decision(
                candidate,
                "omitted",
                state::blocked_reason(authority, &blocked),
                bundle,
                0,
            ));
            continue;
        }
        let mut tentative = included.clone();
        tentative.extend(bundle.iter().cloned());
        let tentative_tokens = measure(&tentative);
        let marginal_tokens = tentative_tokens.saturating_sub(token_count);
        if tentative_tokens <= budget {
            let reason = if marginal_tokens == 0 {
                "already_included"
            } else {
                "fits_budget"
            };
            included = tentative;
            token_count = tentative_tokens;
            decisions.push(decision(
                candidate,
                "included",
                reason,
                bundle,
                marginal_tokens,
            ));
        } else {
            decisions.push(decision(
                candidate,
                "omitted",
                "budget_exceeded",
                bundle,
                marginal_tokens,
            ));
        }
    }

    let observations = observed
        .into_iter()
        .map(|id| state::observation(authority, id))
        .collect();
    Ok(Selection {
        required_roots: required_roots.into_iter().collect(),
        anchors: resolved_anchors,
        included,
        decisions,
        observations,
        token_count,
    })
}

fn decision(
    candidate: &Candidate,
    disposition: &'static str,
    reason: &str,
    bundle: BTreeSet<String>,
    marginal_tokens: usize,
) -> CandidateDecision {
    CandidateDecision {
        candidate: candidate.clone(),
        disposition,
        reason: reason.to_owned(),
        bundle: bundle.into_iter().collect(),
        marginal_tokens,
    }
}

fn failure(
    authority: &ContextAuthority,
    code: &str,
    message: String,
    affected_ids: Vec<String>,
    next_action: &str,
) -> ResolveFailure {
    ResolveFailure::new(
        Some(authority.trusted_commit.clone()),
        code,
        message,
        false,
        affected_ids,
        Vec::new(),
        next_action,
    )
}
