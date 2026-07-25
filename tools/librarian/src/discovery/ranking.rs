//! Inspectable lexical signals and deterministic ordering.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    catalog::{Catalog, Unit},
    wire::{Anchor, Candidate, CandidateReason, DiscoveryRequest},
};

const SCORE_EXACT_ID: u64 = 10_000;
const SCORE_EXACT_ANCHOR: u64 = 8_000;
const SCORE_PHRASE: u64 = 1_000;
const SCORE_TOKEN: u64 = 100;
const SCORE_RELATION: u64 = 10;

#[derive(Default)]
struct Evidence {
    score: u64,
    reasons: BTreeSet<CandidateReason>,
}

pub(super) fn rank(request: &DiscoveryRequest, catalog: &Catalog) -> (Vec<Candidate>, Vec<Anchor>) {
    let mut evidence = BTreeMap::<String, Evidence>::new();
    let mut unresolved = Vec::new();
    for anchor in &request.anchors {
        if !match_anchor(anchor, catalog, &mut evidence) {
            unresolved.push(anchor.clone());
        }
    }
    if let Some(query) = request
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
    {
        for unit in catalog.units.values() {
            score_query(query, unit, evidence.entry(unit.id.clone()).or_default());
        }
    }
    evidence.retain(|_, value| value.score > 0);
    let primary_ids = evidence.keys().cloned().collect::<BTreeSet<_>>();
    add_one_hop_relations(catalog, &primary_ids, &mut evidence);

    let mut candidates = evidence
        .into_iter()
        .map(|(id, evidence)| {
            let unit = &catalog.units[&id];
            Candidate {
                id,
                path: unit.path.clone(),
                score: evidence.score,
                reasons: evidence.reasons.into_iter().collect(),
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.id.cmp(&right.id))
    });
    (candidates, unresolved)
}

fn match_anchor(
    anchor: &Anchor,
    catalog: &Catalog,
    evidence: &mut BTreeMap<String, Evidence>,
) -> bool {
    let value = anchor.value().trim();
    let mut matched = false;
    for unit in catalog.units.values() {
        let field = match anchor {
            Anchor::KnowledgeId { .. } if unit.id == value => Some("id"),
            Anchor::Path { .. } if unit.path == value => Some("path"),
            Anchor::Path { .. } | Anchor::Symbol { .. }
                if unit
                    .relations
                    .applies_to
                    .iter()
                    .any(|target| target == value) =>
            {
                Some("applies_to")
            },
            _ => None,
        };
        if let Some(field) = field {
            matched = true;
            let score = if matches!(anchor, Anchor::KnowledgeId { .. }) {
                SCORE_EXACT_ID
            } else {
                SCORE_EXACT_ANCHOR
            };
            let candidate = evidence.entry(unit.id.clone()).or_default();
            candidate.score += score;
            candidate.reasons.insert(CandidateReason::Anchor {
                anchor_kind: anchor.kind(),
                value: value.to_owned(),
                field,
                score,
            });
        }
    }
    matched
}

fn score_query(query: &str, unit: &Unit, evidence: &mut Evidence) {
    let normalized_query = query.to_lowercase();
    let query_tokens = tokens(&normalized_query);
    let exact_id = normalized_query == unit.id;
    if exact_id {
        evidence.score += SCORE_EXACT_ID;
        evidence.reasons.insert(CandidateReason::ExactQuery {
            field: "id",
            score: SCORE_EXACT_ID,
        });
    }
    for (field, value) in unit.searchable_fields() {
        if exact_id && field == "id" {
            continue;
        }
        score_field(
            field,
            &value.to_lowercase(),
            &normalized_query,
            &query_tokens,
            evidence,
        );
    }
    for (relation, targets) in unit.relations.typed() {
        if !targets.is_empty() {
            score_field(
                relation,
                &targets.join("\n").to_lowercase(),
                &normalized_query,
                &query_tokens,
                evidence,
            );
        }
    }
}

fn score_field(
    field: &'static str,
    value: &str,
    query: &str,
    query_tokens: &BTreeSet<String>,
    evidence: &mut Evidence,
) {
    if value.contains(query) {
        evidence.score += SCORE_PHRASE;
        evidence.reasons.insert(CandidateReason::QueryPhrase {
            field,
            score: SCORE_PHRASE,
        });
    }
    let value_tokens = tokens(value);
    for term in query_tokens.intersection(&value_tokens) {
        evidence.score += SCORE_TOKEN;
        evidence.reasons.insert(CandidateReason::QueryToken {
            field,
            term: term.clone(),
            score: SCORE_TOKEN,
        });
    }
}

fn add_one_hop_relations(
    catalog: &Catalog,
    primary_ids: &BTreeSet<String>,
    evidence: &mut BTreeMap<String, Evidence>,
) {
    let mut edges = BTreeSet::new();
    for unit in catalog.units.values() {
        for target in unit.relations.knowledge_targets() {
            if primary_ids.contains(&unit.id) {
                edges.insert((target.clone(), unit.id.clone()));
            }
            if primary_ids.contains(target) {
                edges.insert((unit.id.clone(), target.clone()));
            }
        }
    }
    for (candidate_id, via) in edges {
        let candidate = evidence.entry(candidate_id).or_default();
        candidate.score += SCORE_RELATION;
        candidate.reasons.insert(CandidateReason::Relation {
            via,
            score: SCORE_RELATION,
        });
    }
}

fn tokens(value: &str) -> BTreeSet<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}
