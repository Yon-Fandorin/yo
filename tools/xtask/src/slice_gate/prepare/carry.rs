use std::path::Path;

use super::model::{
    CanonicalApprovalReviewCarry, CanonicalApprovalReviewCarryResult, REVIEW_CARRY_SCHEMA,
};
use crate::{
    git,
    review_packet::VerifiedReview,
    review_protocol::{self, digest},
};

type ValidateApproval<'a> = dyn Fn(
        &Path,
        &str,
        &[u8],
        Option<&[u8]>,
    ) -> Result<methexis::CanonicalApprovalFollowthrough, Vec<methexis::Diagnostic>>
    + 'a;

pub(super) fn verify(
    repository: &Path,
    review: &VerifiedReview,
    candidate: &str,
    request: &CanonicalApprovalReviewCarry,
) -> Result<CanonicalApprovalReviewCarryResult, String> {
    verify_with(
        repository,
        review,
        candidate,
        request,
        &methexis::validate_canonical_approval_followthrough,
    )
}

fn verify_with(
    repository: &Path,
    review: &VerifiedReview,
    candidate: &str,
    request: &CanonicalApprovalReviewCarry,
    validate_approval: &ValidateApproval<'_>,
) -> Result<CanonicalApprovalReviewCarryResult, String> {
    if request.schema != REVIEW_CARRY_SCHEMA {
        return Err(format!(
            "unsupported canonical approval review carry schema `{}`; expected `{REVIEW_CARRY_SCHEMA}`",
            request.schema
        ));
    }
    portable_knowledge_id(&request.knowledge_id)?;
    review_protocol::require_commit(
        &review.candidate_commit,
        "canonical approval reviewed candidate",
    )?;
    if review.candidate_commit == candidate {
        return Err(
            "canonical approval review carry requires a strict descendant candidate".to_owned(),
        );
    }
    if !git::trusted_succeeds_in(
        repository,
        &[
            "merge-base",
            "--is-ancestor",
            &review.candidate_commit,
            candidate,
        ],
    )? {
        return Err(
            "canonical approval candidate is not a descendant of the reviewed candidate".to_owned(),
        );
    }
    if !review
        .review_lenses
        .iter()
        .any(|lens| lens == "fresh-context")
    {
        return Err(
            "canonical approval review carry requires a completed fresh-context review lens"
                .to_owned(),
        );
    }

    let approval_path = format!("methexis/approvals/{}.yaml", request.knowledge_id);
    let transition_paths =
        super::super::changed_paths(repository, &review.candidate_commit, candidate)?;
    if transition_paths != [approval_path.clone()] {
        return Err(format!(
            "canonical approval review carry requires exactly `{approval_path}` after the reviewed candidate"
        ));
    }

    let semantic_paths =
        super::super::changed_paths(repository, &review.base_commit, &review.candidate_commit)?;
    if semantic_paths.iter().any(|path| path == &approval_path) {
        return Err(
            "the semantic review candidate must not already change the carried approval path"
                .to_owned(),
        );
    }
    let knowledge_suffix = format!("/{}.md", request.knowledge_id);
    let knowledge_paths = semantic_paths
        .iter()
        .filter(|path| path.starts_with("methexis/knowledge/"))
        .cloned()
        .collect::<Vec<_>>();
    let [knowledge_path] = knowledge_paths.as_slice() else {
        return Err(
            "the reviewed semantic candidate must change exactly one matching Knowledge file"
                .to_owned(),
        );
    };
    if !knowledge_path.ends_with(&knowledge_suffix) {
        return Err(
            "the reviewed semantic candidate must change exactly one matching Knowledge file"
                .to_owned(),
        );
    }

    let approval_bytes = required_blob(repository, candidate, &approval_path)?;
    let previous_approval_bytes =
        optional_blob(repository, &review.candidate_commit, &approval_path)?;
    let verified = validate_approval(
        repository,
        &request.knowledge_id,
        &approval_bytes,
        previous_approval_bytes.as_deref(),
    )
    .map_err(|diagnostics| {
        diagnostics.first().map_or_else(
            || "Methexis rejected canonical approval follow-through".to_owned(),
            |diagnostic| {
                format!(
                    "Methexis rejected canonical approval follow-through: {}: {}",
                    diagnostic.code, diagnostic.message
                )
            },
        )
    })?;

    let transition_diff = git::trusted_output_bytes_in(
        repository,
        &[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            &review.candidate_commit,
            candidate,
            "--",
        ],
    )?;
    Ok(CanonicalApprovalReviewCarryResult {
        schema: REVIEW_CARRY_SCHEMA,
        knowledge_id: verified.knowledge_id,
        reviewed_candidate: review.candidate_commit.clone(),
        candidate_commit: candidate.to_owned(),
        knowledge_path: knowledge_path.clone(),
        approval_path,
        revision: verified.revision,
        reviewer: verified.reviewer,
        reviewed_at: verified.reviewed_at,
        request_hash: verified.request_hash,
        approval_hash: verified.approval_hash,
        replaced_revision: verified.replaced_revision,
        transition_diff_hash: digest(&transition_diff),
    })
}

fn portable_knowledge_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
    {
        Err("canonical approval review carry requires a portable KnowledgeId".to_owned())
    } else {
        Ok(())
    }
}

fn required_blob(repository: &Path, commit: &str, path: &str) -> Result<Vec<u8>, String> {
    optional_blob(repository, commit, path)?.ok_or_else(|| {
        format!("canonical approval candidate does not contain regular blob `{path}`")
    })
}

fn optional_blob(repository: &Path, commit: &str, path: &str) -> Result<Option<Vec<u8>>, String> {
    let listing = git::trusted_output_bytes_in(
        repository,
        &["ls-tree", "-z", "--full-tree", commit, "--", path],
    )?;
    if listing.is_empty() {
        return Ok(None);
    }
    let fields = listing
        .strip_suffix(&[0])
        .ok_or_else(|| "Git tree entry is not NUL terminated".to_owned())?
        .splitn(2, |byte| *byte == b'\t')
        .collect::<Vec<_>>();
    let metadata = fields
        .first()
        .and_then(|value| std::str::from_utf8(value).ok())
        .ok_or_else(|| "Git tree entry metadata is not UTF-8".to_owned())?;
    let recorded_path = fields
        .get(1)
        .and_then(|value| std::str::from_utf8(value).ok())
        .ok_or_else(|| "Git tree entry path is not UTF-8".to_owned())?;
    let metadata = metadata.split_ascii_whitespace().collect::<Vec<_>>();
    if metadata.len() != 3
        || metadata[0] != "100644"
        || metadata[1] != "blob"
        || recorded_path != path
    {
        return Err(format!(
            "`{path}` must be one non-executable regular Git blob"
        ));
    }
    git::trusted_output_bytes_in(repository, &["cat-file", "blob", metadata[2]]).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{review_packet::VerifiedEvidence, test_support::TestRepository};

    struct Fixture {
        repository: TestRepository,
        review: VerifiedReview,
        candidate: String,
        request: CanonicalApprovalReviewCarry,
    }

    impl Fixture {
        fn new(extra_transition_path: Option<&str>) -> Self {
            Self::new_with_paths(None, extra_transition_path)
        }

        fn new_with_paths(
            extra_semantic_path: Option<&str>,
            extra_transition_path: Option<&str>,
        ) -> Self {
            let repository = TestRepository::new("canonical-approval-review-carry");
            repository.write("README", "base\n");
            repository.git(["add", "README"]);
            repository.git(["commit", "--quiet", "-m", "base"]);
            let base = git_line(&repository, &["rev-parse", "HEAD"]);

            repository.write(
                "methexis/knowledge/agent-runtime/example.unit.md",
                "---\nid: example.unit\n---\nsemantic candidate\n",
            );
            if let Some(path) = extra_semantic_path {
                repository.write(path, "another semantic change\n");
            }
            repository.git(["add", "methexis/knowledge"]);
            repository.git(["commit", "--quiet", "-m", "semantic candidate"]);
            let reviewed_candidate = git_line(&repository, &["rev-parse", "HEAD"]);

            repository.write(
                "methexis/approvals/example.unit.yaml",
                "canonical approval\n",
            );
            repository.git(["add", "methexis/approvals/example.unit.yaml"]);
            if let Some(path) = extra_transition_path {
                repository.write(path, "unrelated\n");
                repository.git(["add", path]);
            }
            repository.git(["commit", "--quiet", "-m", "canonical approval"]);
            let candidate = git_line(&repository, &["rev-parse", "HEAD"]);

            let review = VerifiedReview {
                review_id: digest(b"review"),
                manifest_path: "manifest.json".to_owned(),
                manifest_hash: digest(b"manifest"),
                packet_path: "packet.md".to_owned(),
                packet_hash: digest(b"packet"),
                base_commit: base.clone(),
                candidate_commit: reviewed_candidate,
                trusted_commit: base,
                slice_contract_path: "slice-contract.json".to_owned(),
                slice_contract_hash: digest(b"contract"),
                validation_evidence: vec![VerifiedEvidence {
                    name: "xtask".to_owned(),
                    path: "validation.json".to_owned(),
                    hash: digest(b"validation"),
                }],
                review_lenses: vec!["fresh-context".to_owned(), "code-quality".to_owned()],
                review_questions: vec!["Is the semantic candidate clear?".to_owned()],
            };
            Self {
                repository,
                review,
                candidate,
                request: CanonicalApprovalReviewCarry {
                    schema: REVIEW_CARRY_SCHEMA.to_owned(),
                    knowledge_id: "example.unit".to_owned(),
                },
            }
        }

        fn verify(&self) -> Result<CanonicalApprovalReviewCarryResult, String> {
            verify_with(
                &self.repository.path,
                &self.review,
                &self.candidate,
                &self.request,
                &|repository, knowledge_id, approval, previous| {
                    assert_eq!(repository, self.repository.path);
                    assert_eq!(knowledge_id, "example.unit");
                    assert_eq!(approval, b"canonical approval\n");
                    assert_eq!(previous, None);
                    Ok(methexis::CanonicalApprovalFollowthrough {
                        knowledge_id: knowledge_id.to_owned(),
                        revision: digest(b"knowledge"),
                        reviewer: "human/owner".to_owned(),
                        reviewed_at: "2026-08-26".to_owned(),
                        request_hash: digest(b"request"),
                        approval_hash: digest(approval),
                        replaced_revision: None,
                    })
                },
            )
        }
    }

    // 승인 경로 하나만 더한 strict descendant는 기존 semantic 리뷰를 carry할 수 있다.
    #[test]
    fn accepts_exact_canonical_approval_only_descendant() {
        let fixture = Fixture::new(None);
        let result = fixture.verify().unwrap();

        assert_eq!(result.schema, REVIEW_CARRY_SCHEMA);
        assert_eq!(result.reviewed_candidate, fixture.review.candidate_commit);
        assert_eq!(result.candidate_commit, fixture.candidate);
        assert_eq!(
            result.knowledge_path,
            "methexis/knowledge/agent-runtime/example.unit.md"
        );
        assert_eq!(result.approval_path, "methexis/approvals/example.unit.yaml");
        assert!(result.transition_diff_hash.starts_with("sha256:"));
    }

    // 승인 뒤에 unrelated 경로가 하나라도 섞이면 기계적 후속 변경 경계를 벗어난다.
    #[test]
    fn rejects_any_other_descendant_change() {
        let fixture = Fixture::new(Some("docs/unrelated.md"));
        assert_eq!(
            fixture.verify().unwrap_err(),
            "canonical approval review carry requires exactly `methexis/approvals/example.unit.yaml` after the reviewed candidate"
        );
    }

    // 한 리뷰 후보가 여러 Knowledge를 바꾸면 단일 승인 대상으로 리뷰 범위를 축소하지 않는다.
    #[test]
    fn rejects_a_semantic_candidate_that_changes_another_knowledge_file() {
        let fixture = Fixture::new_with_paths(
            Some("methexis/knowledge/agent-runtime/another.unit.md"),
            None,
        );
        assert_eq!(
            fixture.verify().unwrap_err(),
            "the reviewed semantic candidate must change exactly one matching Knowledge file"
        );
    }

    // KnowledgeId는 승인 경로를 만들기 전에 휴대 가능한 단일 식별자로 제한한다.
    #[test]
    fn rejects_nonportable_knowledge_id_before_git_lookup() {
        let mut fixture = Fixture::new(None);
        fixture.request.knowledge_id = "../example.unit".to_owned();
        assert_eq!(
            fixture.verify().unwrap_err(),
            "canonical approval review carry requires a portable KnowledgeId"
        );
    }

    fn git_line(repository: &TestRepository, arguments: &[&str]) -> String {
        git::output_in(&repository.path, arguments, false)
            .unwrap()
            .trim()
            .to_owned()
    }
}
