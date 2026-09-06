use super::{ImpactInput, developer_docs, preflight};

/// Ordinary commits have no mandatory protocol metadata. Explicit review
/// claims and Wave integration still receive the existing strict checks.
pub(crate) fn check(input: &ImpactInput) -> Result<(), String> {
    let has_trailer = |name: &str| {
        input.message.lines().any(|line| {
            line.trim_start()
                .split_once(':')
                .is_some_and(|(key, _)| key.eq_ignore_ascii_case(name))
        })
    };
    if input.branch.starts_with("wave/")
        || has_trailer("Slice-Review")
        || has_trailer("Review-Coverage")
    {
        return preflight::check(input);
    }
    if has_trailer("Developer-Docs-Impact") {
        return developer_docs::check(input);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestRepository;

    fn input(repository: &TestRepository, message: &str) -> ImpactInput {
        ImpactInput {
            message: message.to_owned(),
            changed_paths: vec!["tools/xtask/src/lib.rs".to_owned()],
            branch: "develop".to_owned(),
            merge_head: None,
            repository: repository.path.clone(),
            inherit_git_environment: false,
        }
    }

    // 일반 코드 변경은 형식적 Slice 증거 없이 진행하지만 같은 입력을 명시적
    // formal preflight에 넣으면 여전히 누락된 review와 docs 증거를 거부한다.
    #[test]
    fn ordinary_change_does_not_require_formal_evidence() {
        let repository = TestRepository::new("ordinary-change");
        let input = input(&repository, "fix: clarify command diagnostic\n");
        assert!(check(&input).is_ok());
        assert!(preflight::check(&input).is_err());
    }

    // 선택적으로 쓴 review 주장도 검사하므로 code 변경에 none을 붙이거나
    // coverage만 적어 formal review를 완료한 것처럼 표시할 수 없다.
    #[test]
    fn supplied_review_claims_are_not_silently_ignored() {
        let repository = TestRepository::new("ordinary-review-claim");
        for claim in [
            "Slice-Review: none - ordinary change",
            "Review-Coverage: fabricated",
            "slice-review: none - lowercase claim",
        ] {
            assert!(check(&input(&repository, &format!("fix: claim\n\n{claim}\n"))).is_err());
        }
    }

    // 문서 갱신을 선언했다면 실제 docs 변경이 있어야 하고, 영향 없음의 구체적
    // 설명은 별도 Slice review를 요구하지 않고 그대로 허용한다.
    #[test]
    fn optional_documentation_claim_must_match_the_change() {
        let repository = TestRepository::new("ordinary-docs-claim");
        assert!(
            check(&input(
                &repository,
                "fix: docs\n\nDeveloper-Docs-Impact: updated\n"
            ))
            .is_err()
        );
        assert!(
            check(&input(
                &repository,
                "fix: diagnostic\n\nDeveloper-Docs-Impact: none - same command and owner\n"
            ))
            .is_ok()
        );
    }

    // Wave는 이미 선택한 formal coordination 경계이므로 일반 hook으로 호출해도
    // 검수 증거가 없는 통합을 통과시키지 않는다.
    #[test]
    fn wave_integration_keeps_formal_requirements() {
        let repository = TestRepository::new("ordinary-wave");
        let mut input = input(&repository, "fix: integrate component\n");
        input.branch = "wave/coordinated".to_owned();
        assert!(check(&input).is_err());
    }
}
