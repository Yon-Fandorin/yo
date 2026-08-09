//! Source freshness for one trusted active Checkpoint.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
};

use super::{Eligibility, FreshnessEvaluation, UnitFreshness};
use crate::{
    check::Foundation,
    model::{Source, SourcePayload},
};

pub(crate) fn evaluate(
    repository_root: &Path,
    trusted: &Foundation,
    working_sources: &[Source],
    selected: &BTreeSet<String>,
) -> Result<FreshnessEvaluation, FreshnessFailure> {
    let (working_negative_records, negative_record_capture) =
        super::negative::load_captured(repository_root)?;
    super::negative::validate_for_evaluation(&working_negative_records, trusted)?;
    let trusted_sources = sources_by_id(&trusted.sources);
    let working_sources_by_id = sources_by_id(working_sources);
    let trusted_units = trusted
        .units
        .iter()
        .map(|unit| (unit.metadata.id.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let mut units = BTreeMap::new();
    let mut code_captures = Vec::new();

    for id in selected {
        let unit = trusted_units
            .get(id.as_str())
            .ok_or_else(|| FreshnessFailure {
                code: "active_unit_missing",
                message: format!("active KnowledgeId `{id}` is absent from trusted foundation"),
                affected_ids: vec![id.clone()],
            })?;
        let mut eligibility = Eligibility::Active;
        let mut evidence = Vec::new();
        for pinned in &unit.metadata.sources {
            let Some(source) = trusted_sources.get(pinned.id.as_str()) else {
                eligibility = Eligibility::Invalid;
                evidence.push(format!("source_missing:{}", pinned.id));
                continue;
            };
            if source.record.revision != pinned.revision {
                eligibility = eligibility.max(Eligibility::Stale);
                evidence.push(format!("source_revision_mismatch:{}", pinned.id));
                continue;
            }
            let Some(working_source) = working_sources_by_id.get(pinned.id.as_str()) else {
                eligibility = eligibility.max(Eligibility::Stale);
                evidence.push(format!("working_source_missing:{}", pinned.id));
                continue;
            };
            if working_source.record.revision != pinned.revision {
                eligibility = eligibility.max(Eligibility::Stale);
                evidence.push(format!("working_source_drift:{}", pinned.id));
                continue;
            }
            match &source.record.payload {
                SourcePayload::Decision { .. } => {
                    evidence.push(format!("decision_revision_match:{}", pinned.id));
                },
                SourcePayload::Code {
                    path, content_hash, ..
                } => match super::working_tree::capture(repository_root, path, content_hash)
                    .map_err(|failure| with_affected(failure, id, &pinned.id))?
                {
                    super::working_tree::CaptureState::Fresh(capture) => {
                        evidence.push(format!("code_hash_match:{}", pinned.id));
                        code_captures.push((capture, id.clone(), pinned.id.clone()));
                    },
                    super::working_tree::CaptureState::Stale { reason, capture } => {
                        eligibility = eligibility.max(Eligibility::Stale);
                        evidence.push(format!("{reason}:{}", pinned.id));
                        code_captures.push((capture, id.clone(), pinned.id.clone()));
                    },
                    super::working_tree::CaptureState::Invalid { reason, capture } => {
                        eligibility = Eligibility::Invalid;
                        evidence.push(format!("{reason}:{}", pinned.id));
                        code_captures.push((capture, id.clone(), pinned.id.clone()));
                    },
                },
                SourcePayload::Conversation { .. } => {
                    eligibility = eligibility.max(Eligibility::Stale);
                    evidence.push(format!("conversation_unverified:{}", pinned.id));
                },
                SourcePayload::External { .. } => {
                    eligibility = eligibility.max(Eligibility::Stale);
                    evidence.push(format!("external_unverified:{}", pinned.id));
                },
            }
        }
        evidence.sort();
        evidence.dedup();
        units.insert(
            id.clone(),
            UnitFreshness {
                eligibility,
                evidence,
            },
        );
    }

    super::negative::apply(
        &trusted.negative_records,
        &working_negative_records,
        &trusted_units,
        selected,
        &mut units,
    );
    propagate_required_dependents(&trusted_units, selected, &mut units);
    let guard = FreshnessGuard {
        source_revisions: source_revisions(working_sources),
        code_captures,
        record_captures: Vec::new(),
        negative_record_capture: Some(negative_record_capture),
    };
    final_revalidate(repository_root, &guard)?;
    let checkpoint = if units
        .values()
        .all(|state| state.eligibility == Eligibility::Active)
    {
        "active"
    } else {
        "degraded"
    };
    Ok(FreshnessEvaluation {
        units,
        checkpoint,
        guard,
    })
}

pub(crate) struct FreshnessGuard {
    source_revisions: BTreeMap<String, String>,
    code_captures: Vec<(super::working_tree::Capture, String, String)>,
    record_captures: Vec<super::working_tree::Capture>,
    negative_record_capture: Option<super::working_tree::Capture>,
}

impl FreshnessGuard {
    pub(crate) fn empty() -> Self {
        Self {
            source_revisions: BTreeMap::new(),
            code_captures: Vec::new(),
            record_captures: Vec::new(),
            negative_record_capture: None,
        }
    }

    pub(crate) fn add_record_captures(&mut self, captures: Vec<super::working_tree::Capture>) {
        self.record_captures = captures;
    }
}

pub(crate) fn final_revalidate(
    repository_root: &Path,
    guard: &FreshnessGuard,
) -> Result<(), FreshnessFailure> {
    if let Some(capture) = &guard.negative_record_capture {
        super::working_tree::final_revalidate(repository_root, capture).map_err(|_| {
            FreshnessFailure {
                code: "negative_records_changed_during_validation",
                message: "negative-record input changed during validation".to_owned(),
                affected_ids: Vec::new(),
            }
        })?;
    }
    final_revalidate_source_records(repository_root, &guard.source_revisions)?;
    for capture in &guard.record_captures {
        super::working_tree::final_revalidate(repository_root, capture)?;
    }
    for (capture, knowledge_id, source_id) in &guard.code_captures {
        super::working_tree::final_revalidate(repository_root, capture)
            .map_err(|failure| with_affected(failure, knowledge_id, source_id))?;
    }
    Ok(())
}

fn with_affected(
    mut failure: FreshnessFailure,
    knowledge_id: &str,
    source_id: &str,
) -> FreshnessFailure {
    failure.affected_ids = vec![knowledge_id.to_owned(), source_id.to_owned()];
    failure
}

fn sources_by_id(sources: &[Source]) -> BTreeMap<&str, &Source> {
    sources
        .iter()
        .map(|source| (source.record.id.as_str(), source))
        .collect()
}

pub(super) fn propagate_required_dependents(
    trusted_units: &BTreeMap<&str, &crate::model::KnowledgeUnit>,
    selected: &BTreeSet<String>,
    states: &mut BTreeMap<String, UnitFreshness>,
) {
    let mut reverse = BTreeMap::<&str, Vec<&str>>::new();
    for id in selected {
        let Some(unit) = trusted_units.get(id.as_str()) else {
            continue;
        };
        for target in unit.metadata.relations.required_targets() {
            if selected.contains(target) {
                reverse.entry(target).or_default().push(id);
            }
        }
    }
    let mut queue = states
        .iter()
        .filter(|(_, state)| state.eligibility != Eligibility::Active)
        .map(|(id, _)| id.clone())
        .collect::<VecDeque<_>>();
    while let Some(blocked) = queue.pop_front() {
        let blocked_state = states[&blocked].eligibility;
        for dependent in reverse.get(blocked.as_str()).into_iter().flatten().copied() {
            let state = states
                .get_mut(dependent)
                .expect("selected dependent has a freshness state");
            if state.eligibility < blocked_state {
                state.eligibility = blocked_state;
                state.evidence.push(format!(
                    "required_knowledge_state:{}:{blocked}",
                    blocked_state.as_str()
                ));
                state.evidence.sort();
                state.evidence.dedup();
                queue.push_back(dependent.to_owned());
            }
        }
    }
}

fn source_revisions(sources: &[Source]) -> BTreeMap<String, String> {
    sources
        .iter()
        .map(|source| (source.record.id.clone(), source.record.revision.clone()))
        .collect()
}

fn final_revalidate_source_records(
    repository_root: &Path,
    initial: &BTreeMap<String, String>,
) -> Result<(), FreshnessFailure> {
    let (final_sources, _) =
        super::records::load_captured(repository_root).map_err(|diagnostics| FreshnessFailure {
            code: "source_changed_during_validation",
            message: diagnostics.first().map_or_else(
                || "Source records changed during validation".to_owned(),
                |diagnostic| diagnostic.message.clone(),
            ),
            affected_ids: diagnostics
                .into_iter()
                .flat_map(|diagnostic| diagnostic.affected_ids)
                .collect(),
        })?;
    let final_state = source_revisions(&final_sources);
    if *initial != final_state {
        return Err(FreshnessFailure {
            code: "source_changed_during_validation",
            message: "Source records changed during validation".to_owned(),
            affected_ids: initial
                .keys()
                .chain(final_state.keys())
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        });
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct FreshnessFailure {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) affected_ids: Vec<String>,
}
