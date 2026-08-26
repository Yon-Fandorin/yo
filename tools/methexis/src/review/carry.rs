use std::path::Path;

use super::{
    ApprovalBasis, ApprovalRequest, CanonicalBasis,
    records::{parse_approval_bytes, render_approval},
};
use crate::{
    CanonicalApprovalFollowthrough,
    check::{Diagnostic, load_foundation},
};

pub(crate) fn validate_canonical_approval_followthrough(
    repository_root: &Path,
    knowledge_id: &str,
    approval_bytes: &[u8],
    previous_approval_bytes: Option<&[u8]>,
) -> Result<CanonicalApprovalFollowthrough, Vec<Diagnostic>> {
    let foundation = load_foundation(repository_root)?;
    let Some(unit) = foundation
        .units
        .iter()
        .find(|unit| unit.metadata.id == knowledge_id)
    else {
        return Err(vec![diagnostic(
            approval_path(knowledge_id),
            "unknown_knowledge_id",
            format!("canonical approval targets unknown KnowledgeId `{knowledge_id}`"),
            knowledge_id,
        )]);
    };
    let path = repository_root.join(approval_path(knowledge_id));
    let approval = parse_approval_bytes(approval_bytes, &path, repository_root)
        .map_err(|error| vec![error])?;
    if approval.knowledge_id != knowledge_id
        || approval.basis != ApprovalBasis::Canonical
        || approval.revision != unit.revision
        || approval.review_hash != unit.revision
        || !foundation
            .owners
            .iter()
            .any(|owner| owner.id == approval.reviewer)
    {
        return Err(vec![diagnostic(
            approval_path(knowledge_id),
            "canonical_approval_followthrough_mismatch",
            "approval must bind the requested KnowledgeId, its current exact revision, canonical review basis, and a tracked reviewer OwnerId".to_owned(),
            knowledge_id,
        )]);
    }

    let previous = match previous_approval_bytes {
        Some(bytes) => {
            Some(parse_approval_bytes(bytes, &path, repository_root).map_err(|error| vec![error])?)
        },
        None => None,
    };
    if previous
        .as_ref()
        .is_some_and(|record| record.knowledge_id != knowledge_id)
    {
        return Err(vec![diagnostic(
            approval_path(knowledge_id),
            "canonical_approval_replacement_mismatch",
            "previous approval must belong to the same KnowledgeId".to_owned(),
            knowledge_id,
        )]);
    }
    if previous_approval_bytes == Some(approval_bytes) {
        return Err(vec![diagnostic(
            approval_path(knowledge_id),
            "canonical_approval_transition_empty",
            "canonical approval follow-through must change the approval record".to_owned(),
            knowledge_id,
        )]);
    }

    let request = ApprovalRequest::Canonical {
        knowledge_id: approval.knowledge_id.clone(),
        expected_revision: approval.revision.clone(),
        review_basis: CanonicalBasis::Canonical,
        reviewer: approval.reviewer.clone(),
        reviewed_at: approval.reviewed_at.clone(),
        replace_revision: previous.as_ref().map(|record| record.revision.clone()),
    };
    let expected = render_approval(&request, None, &approval.request_hash);
    if approval_bytes != expected {
        return Err(vec![diagnostic(
            approval_path(knowledge_id),
            "canonical_approval_output_not_exact",
            "approval bytes are valid but are not the exact deterministic canonical output"
                .to_owned(),
            knowledge_id,
        )]);
    }

    Ok(CanonicalApprovalFollowthrough {
        knowledge_id: approval.knowledge_id,
        revision: approval.revision,
        reviewer: approval.reviewer,
        reviewed_at: approval.reviewed_at,
        request_hash: approval.request_hash,
        approval_hash: approval.hash,
        replaced_revision: previous.map(|record| record.revision),
    })
}

fn approval_path(knowledge_id: &str) -> String {
    format!("methexis/approvals/{knowledge_id}.yaml")
}

fn diagnostic(path: String, code: &str, message: String, knowledge_id: &str) -> Diagnostic {
    Diagnostic {
        phase: crate::DiagnosticPhase::Local,
        path,
        code: code.to_owned(),
        message,
        line: None,
        column: None,
        affected_ids: vec![knowledge_id.to_owned()],
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::validate_canonical_approval_followthrough;
    use crate::review::{
        ApprovalRequest, CanonicalApprovalInput, CanonicalBasis, records::render_approval,
        semantic_hash,
    };

    fn repository() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    // 현재 corpus의 canonical approval을 다시 계산해 exact bytes와 revision lineage를 묶는다.
    #[test]
    fn exact_canonical_record_rederives_its_request_and_output() {
        let repository = repository();
        let knowledge_id = "agent.delivery.first-coding-loop";
        let bytes = fs::read(
            repository
                .join("methexis/approvals")
                .join(format!("{knowledge_id}.yaml")),
        )
        .unwrap();

        let verified =
            validate_canonical_approval_followthrough(&repository, knowledge_id, &bytes, None)
                .unwrap();

        assert_eq!(verified.knowledge_id, knowledge_id);
        assert!(verified.revision.starts_with("sha256:"));
        assert!(verified.request_hash.starts_with("sha256:"));
        assert!(verified.approval_hash.starts_with("sha256:"));
        assert_eq!(verified.replaced_revision, None);
    }

    // 의미상 같은 YAML이라도 operation이 만든 canonical bytes가 아니면 carry 근거가 될 수 없다.
    #[test]
    fn noncanonical_yaml_rendering_is_rejected() {
        let repository = repository();
        let knowledge_id = "agent.delivery.first-coding-loop";
        let path = repository
            .join("methexis/approvals")
            .join(format!("{knowledge_id}.yaml"));
        let mut bytes = fs::read(path).unwrap();
        bytes.push(b'\n');

        let errors =
            validate_canonical_approval_followthrough(&repository, knowledge_id, &bytes, None)
                .unwrap_err();

        assert_eq!(errors[0].code, "canonical_approval_output_not_exact");
    }

    // 기존 승인 blob을 replacement precondition으로 사용해 새 canonical proposal의 계보를 보존한다.
    #[test]
    fn replacement_derives_the_previous_approval_revision() {
        let repository = repository();
        let knowledge_id = "agent.delivery.first-coding-loop";
        let path = repository
            .join("methexis/approvals")
            .join(format!("{knowledge_id}.yaml"));
        let previous_bytes = fs::read(&path).unwrap();
        let previous = super::parse_approval_bytes(&previous_bytes, &path, &repository).unwrap();
        let request = ApprovalRequest::Canonical {
            knowledge_id: knowledge_id.to_owned(),
            expected_revision: previous.revision.clone(),
            review_basis: CanonicalBasis::Canonical,
            reviewer: previous.reviewer.clone(),
            reviewed_at: "2026-08-27T00:00:00Z".to_owned(),
            replace_revision: Some(previous.revision.clone()),
        };
        let request_hash = semantic_hash(&CanonicalApprovalInput {
            schema: super::super::CANONICAL_APPROVAL_REQUEST_SCHEMA,
            knowledge_id,
            expected_revision: &previous.revision,
            review_basis: CanonicalBasis::Canonical,
            reviewer: &previous.reviewer,
            reviewed_at: "2026-08-27T00:00:00Z",
        });
        let current_bytes = render_approval(&request, None, &request_hash);

        let verified = validate_canonical_approval_followthrough(
            &repository,
            knowledge_id,
            &current_bytes,
            Some(&previous_bytes),
        )
        .unwrap();

        assert_eq!(verified.replaced_revision, Some(previous.revision));
    }
}
