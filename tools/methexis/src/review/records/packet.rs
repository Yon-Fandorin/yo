//! 결정적 review packet 본문을 렌더링한다.

use crate::{model::KnowledgeUnit, review::ProjectionRecord};

pub(crate) fn render_review_packet(unit: &KnowledgeUnit, projection: &ProjectionRecord) -> String {
    let sources = unit
        .metadata
        .sources
        .iter()
        .map(|source| {
            format!(
                "- `{}` at `{}` — `not_evaluated`",
                source.id, source.revision
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let relations = unit
        .metadata
        .relations
        .typed()
        .into_iter()
        .flat_map(|(kind, targets)| {
            targets
                .iter()
                .map(move |target| format!("- `{kind}` → `{target}`"))
        })
        .collect::<Vec<_>>();
    let relations = if relations.is_empty() {
        "- none".to_owned()
    } else {
        relations.join("\n")
    };
    format!(
        "# Methexis Review Packet\n\n- KnowledgeId: `{}`\n- RevisionId: `{}`\n- OwnerId: `{}`\n- Projection profile: `{}`\n- Projection compiler: `{}`\n- Projection hash: `{}`\n- Source validation: `not_evaluated`\n\n## Canonical English\n\n{}\n\n## Korean Review Projection\n\n{}\n\n## Source references\n\n{}\n\n## Relations\n\n{}\n",
        unit.metadata.id,
        unit.revision,
        unit.metadata.owner,
        projection.metadata.profile,
        projection.metadata.compiler,
        projection.hash,
        unit.body.trim_end(),
        projection.body.trim_end(),
        sources,
        relations,
    )
}
