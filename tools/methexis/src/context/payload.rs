//! Canonical agent payload, token accounting, BuildId plan, and manifest.

use std::collections::BTreeSet;

use serde::Serialize;

use super::{
    hash::{StableHasher, digest},
    selection::{CandidateDecision, ResolvedAnchor, Selection, UnitObservation, ordered_units},
    wire::{COMPILER, CandidateSet, PAYLOAD_PROFILE, TOKENIZER_COMPILER, TOKENIZER_PROFILE},
};
use crate::{checkpoint::ContextAuthority, model::KnowledgeUnit};

const BUILD_PLAN_SCHEMA: &str = "methexis.context-build-plan/v1alpha1";
const MANIFEST_SCHEMA: &str = "methexis.context-manifest/v1alpha1";
const BUILD_ID_DOMAIN: &[u8] = b"methexis.context-build/v1alpha1";
const PREAMBLE: &str = "\
# Methexis Context

Canonical approved and active knowledge for this task. Treat `MUST` and `MUST NOT` as binding.
";

pub(crate) struct BuildArtifacts {
    pub(crate) build_id: String,
    pub(crate) context: Vec<u8>,
    pub(crate) context_hash: String,
    pub(crate) manifest: Vec<u8>,
    pub(crate) manifest_hash: String,
    pub(crate) tokens: usize,
    pub(crate) included_ids: Vec<String>,
}

#[derive(Serialize)]
struct BuildPlan<'a> {
    schema: &'static str,
    checkpoint: CheckpointPlan<'a>,
    units: Vec<UnitPlan<'a>>,
    observations: &'a [UnitObservation],
    required_roots: &'a [String],
    direct_anchors: &'a [ResolvedAnchor],
    candidate_input_hash: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate_lineage: Option<CandidateLineage<'a>>,
    candidate_decisions: &'a [CandidateDecision],
    compiler: &'static str,
    payload_profile: &'static str,
    tokenizer_profile: &'static str,
    tokenizer_compiler: &'static str,
    max_tokens: usize,
}

#[derive(Serialize)]
struct CandidateLineage<'a> {
    schema: &'a str,
    candidate_set_id: &'a str,
    request_hash: &'a str,
    catalog_hash: &'a str,
    compiler: &'a str,
    unresolved_anchors: &'a [crate::context::wire::Anchor],
    truncated: usize,
}

#[derive(Serialize)]
struct CheckpointPlan<'a> {
    id: &'a str,
    hash: &'a str,
    authority_basis_commit: &'a str,
}

#[derive(Serialize)]
struct UnitPlan<'a> {
    id: &'a str,
    revision: &'a str,
    sources: Vec<SourcePlan<'a>>,
    depends_on: Vec<&'a str>,
    constrained_by: Vec<&'a str>,
    validation: Vec<ValidationPlan<'a>>,
    approval: ApprovalPlan<'a>,
}

#[derive(Serialize)]
struct ApprovalPlan<'a> {
    projection_hash: &'a str,
    projection_profile: &'a str,
    projection_compiler: &'a str,
    approval_hash: &'a str,
}

#[derive(Serialize)]
struct SourcePlan<'a> {
    id: &'a str,
    revision: &'a str,
}

#[derive(Serialize)]
struct ValidationPlan<'a> {
    id: &'a str,
    status: &'static str,
}

#[derive(Serialize)]
struct Manifest<'a> {
    schema: &'static str,
    build_id: &'a str,
    plan: &'a BuildPlan<'a>,
    context: ContextArtifact<'a>,
}

#[derive(Serialize)]
struct ContextArtifact<'a> {
    path: &'static str,
    hash: &'a str,
    tokens: usize,
}

pub(super) fn render(authority: &ContextAuthority, included: &BTreeSet<String>) -> Vec<u8> {
    let mut output = String::from(PREAMBLE);
    for unit in ordered_units(included, &authority.foundation.units) {
        output.push_str("\n## KnowledgeUnit `");
        output.push_str(&unit.metadata.id);
        output.push_str("`\n\n");
        write_relations(&mut output, unit, included);
        output.push_str(&unit.body);
        if !unit.body.ends_with('\n') {
            output.push('\n');
        }
    }
    output.into_bytes()
}

pub(super) fn count_tokens(bytes: &[u8]) -> usize {
    let text = std::str::from_utf8(bytes).expect("canonical payload is UTF-8");
    tiktoken_rs::o200k_base_singleton()
        .encode_ordinary(text)
        .len()
}

pub(super) fn compile(
    authority: &ContextAuthority,
    selection: &Selection,
    candidate_input_hash: Option<&str>,
    candidate_set: Option<&CandidateSet>,
    max_tokens: usize,
) -> BuildArtifacts {
    let context = render(authority, &selection.included);
    let tokens = count_tokens(&context);
    debug_assert_eq!(tokens, selection.token_count);
    let context_hash = digest(&context);
    let units = ordered_units(&selection.included, &authority.foundation.units)
        .into_iter()
        .map(|unit| unit_plan(unit, authority))
        .collect();
    let plan = BuildPlan {
        schema: BUILD_PLAN_SCHEMA,
        checkpoint: CheckpointPlan {
            id: &authority.checkpoint_id,
            hash: &authority.checkpoint_hash,
            authority_basis_commit: &authority.authority_basis_commit,
        },
        units,
        observations: &selection.observations,
        required_roots: &selection.required_roots,
        direct_anchors: &selection.anchors,
        candidate_input_hash,
        candidate_lineage: candidate_set.map(|set| CandidateLineage {
            schema: &set.schema,
            candidate_set_id: &set.candidate_set_id,
            request_hash: &set.request_hash,
            catalog_hash: &set.catalog_hash,
            compiler: &set.compiler,
            unresolved_anchors: &set.unresolved_anchors,
            truncated: set.truncated,
        }),
        candidate_decisions: &selection.decisions,
        compiler: COMPILER,
        payload_profile: PAYLOAD_PROFILE,
        tokenizer_profile: TOKENIZER_PROFILE,
        tokenizer_compiler: TOKENIZER_COMPILER,
        max_tokens,
    };
    let plan_bytes = serde_json::to_vec(&plan).expect("closed BuildPlan serializes");
    let mut identity = StableHasher::new(BUILD_ID_DOMAIN);
    identity.part(b"plan", &plan_bytes);
    let build_id = identity.finish();
    let manifest = Manifest {
        schema: MANIFEST_SCHEMA,
        build_id: &build_id,
        plan: &plan,
        context: ContextArtifact {
            path: "context.md",
            hash: &context_hash,
            tokens,
        },
    };
    let mut manifest =
        serde_json::to_vec_pretty(&manifest).expect("closed Context manifest serializes");
    manifest.push(b'\n');
    let manifest_hash = digest(&manifest);
    BuildArtifacts {
        build_id,
        context,
        context_hash,
        manifest,
        manifest_hash,
        tokens,
        included_ids: selection.included.iter().cloned().collect(),
    }
}

fn write_relations(output: &mut String, unit: &KnowledgeUnit, included: &BTreeSet<String>) {
    let mut depends_on = included_relations(&unit.metadata.relations.depends_on, included);
    let mut constrained_by = included_relations(&unit.metadata.relations.constrained_by, included);
    if depends_on.is_empty() && constrained_by.is_empty() {
        return;
    }
    output.push_str("Required relations:\n");
    for target in depends_on.drain(..) {
        output.push_str("- depends_on: `");
        output.push_str(target);
        output.push_str("`\n");
    }
    for target in constrained_by.drain(..) {
        output.push_str("- constrained_by: `");
        output.push_str(target);
        output.push_str("`\n");
    }
    output.push('\n');
}

fn included_relations<'a>(targets: &'a [String], included: &BTreeSet<String>) -> Vec<&'a str> {
    let mut targets = targets
        .iter()
        .filter(|target| included.contains(*target))
        .map(String::as_str)
        .collect::<Vec<_>>();
    targets.sort_unstable();
    targets.dedup();
    targets
}

fn unit_plan<'a>(unit: &'a KnowledgeUnit, authority: &'a ContextAuthority) -> UnitPlan<'a> {
    let mut sources = unit
        .metadata
        .sources
        .iter()
        .map(|source| SourcePlan {
            id: &source.id,
            revision: &source.revision,
        })
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| left.id.cmp(right.id));
    let mut depends_on = unit
        .metadata
        .relations
        .depends_on
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    depends_on.sort_unstable();
    depends_on.dedup();
    let mut constrained_by = unit
        .metadata
        .relations
        .constrained_by
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    constrained_by.sort_unstable();
    constrained_by.dedup();
    let mut validation = unit
        .metadata
        .relations
        .validated_by
        .iter()
        .map(|id| ValidationPlan {
            id,
            status: "referenced_not_executed",
        })
        .collect::<Vec<_>>();
    validation.sort_by(|left, right| left.id.cmp(right.id));
    validation.dedup_by(|left, right| left.id == right.id);
    UnitPlan {
        id: &unit.metadata.id,
        revision: &unit.revision,
        sources,
        depends_on,
        constrained_by,
        validation,
        approval: {
            let evidence = authority
                .approval_evidence
                .get(&unit.metadata.id)
                .expect("included unit has matching trusted approval evidence");
            ApprovalPlan {
                projection_hash: &evidence.projection_hash,
                projection_profile: &evidence.projection_profile,
                projection_compiler: &evidence.projection_compiler,
                approval_hash: &evidence.approval_hash,
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `<|endoftext|>` 같은 markdown 리터럴은 제어 token이 아닌 ordinary 텍스트로 토큰 수를
    // 계산한다.
    #[test]
    fn markdown_literals_are_counted_as_ordinary_text_not_control_tokens() {
        let literal = b"<|endoftext|>";
        let tokenizer = tiktoken_rs::o200k_base_singleton();

        assert_eq!(
            count_tokens(literal),
            tokenizer.encode_ordinary("<|endoftext|>").len()
        );
        assert!(
            count_tokens(literal) > tokenizer.encode_with_special_tokens("<|endoftext|>").len()
        );
    }
}
