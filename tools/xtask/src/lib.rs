mod docs_translation;
mod git;
mod impact;
mod slice_contract;
mod test_explanations;
mod validation_stage;

#[cfg(test)]
mod test_support;

use std::{ffi::OsString, path::PathBuf};

use impact::ImpactInput;

pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut arguments = arguments.into_iter();
    match (arguments.next().as_deref(), arguments.next().as_deref()) {
        (Some(command), Some(action)) if command == "docs" && action == "accept-translation" => {
            let page = arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(docs_accept_translation_usage)?;
            if arguments.next().is_some() {
                return Err(docs_accept_translation_usage());
            }
            let repository = std::env::current_dir()
                .map_err(|error| format!("cannot locate the repository: {error}"))?;
            docs_translation::accept(&repository, &page)
        },
        (Some(command), Some(action)) if command == "slice-contract" && action == "bind" => {
            let contract = arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(slice_contract_usage)?;
            if arguments.next().is_some() {
                return Err(slice_contract_usage());
            }
            let repository = std::env::current_dir()
                .map_err(|error| format!("cannot locate the repository: {error}"))?;
            slice_contract::bind(&repository, &contract)
        },
        (Some(command), Some(check)) if command == "check" => {
            let check = check.to_string_lossy();
            if check == "test-explanations" {
                if arguments.next().is_some() {
                    return Err(usage(check.as_ref()));
                }
                let repository = std::env::current_dir()
                    .map_err(|error| format!("cannot locate the repository: {error}"))?;
                return test_explanations::check(&repository);
            }
            if check == "slice-scope" {
                let contract = arguments.next().map(PathBuf::from);
                if arguments.next().is_some() {
                    return Err(usage(check.as_ref()));
                }
                let repository = std::env::current_dir()
                    .map_err(|error| format!("cannot locate the repository: {error}"))?;
                return match contract {
                    Some(contract) => slice_contract::check_scope(&repository, &contract),
                    None => slice_contract::check_bound_scope(&repository),
                };
            }
            if check == "slice-parallel" {
                let left = arguments
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| usage(check.as_ref()))?;
                let right = arguments
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| usage(check.as_ref()))?;
                if arguments.next().is_some() {
                    return Err(usage(check.as_ref()));
                }
                let repository = std::env::current_dir()
                    .map_err(|error| format!("cannot locate the repository: {error}"))?;
                return slice_contract::check_parallel(&repository, &left, &right);
            }
            if check == "methexis-check-for-stage" {
                if arguments.next().is_some() {
                    return Err(usage(check.as_ref()));
                }
                let repository = std::env::current_dir()
                    .map_err(|error| format!("cannot locate the repository: {error}"))?;
                return validation_stage::run_methexis_check(&repository);
            }

            let head_fallback =
                matches!(check.as_ref(), "commit-preflight" | "slice-review-impact");
            if !matches!(
                check.as_ref(),
                "commit-preflight" | "developer-docs-impact" | "slice-review-impact"
            ) {
                return Err(usage(check.as_ref()));
            }
            let message = arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| usage(check.as_ref()))?;
            let changed_paths = arguments.next().map(PathBuf::from);
            let branch = arguments
                .next()
                .map(|value| value.to_string_lossy().into_owned());
            if arguments.next().is_some() {
                return Err(usage(check.as_ref()));
            }
            let input = ImpactInput::load(message, changed_paths, branch, head_fallback)?;
            match check.as_ref() {
                "commit-preflight" => impact::preflight::check(&input),
                "developer-docs-impact" => impact::developer_docs::check(&input),
                "slice-review-impact" => impact::slice_review::check(&input),
                _ => unreachable!("the check name was validated before loading input"),
            }
        },
        _ => Err(general_usage()),
    }
}

fn usage(check: &str) -> String {
    match check {
        "test-explanations" | "methexis-check-for-stage" => {
            return format!("usage: cargo xtask check {check}");
        },
        "slice-scope" => {
            return "usage: cargo xtask check slice-scope [slice-contract.json]".to_owned();
        },
        "slice-parallel" => {
            return "usage: cargo xtask check slice-parallel <left.json> <right.json>".to_owned();
        },
        _ => {},
    }
    format!(
        "usage: cargo xtask check {} <commit-message-file> [changed-paths-file] [branch]",
        check
    )
}

fn general_usage() -> String {
    "usage:\n\
     cargo xtask docs accept-translation <relative-page.md>\n\
     cargo xtask slice-contract bind <slice-contract.json>\n\
     cargo xtask check test-explanations\n\
     cargo xtask check methexis-check-for-stage\n\
     cargo xtask check slice-scope [slice-contract.json]\n\
     cargo xtask check slice-parallel <left.json> <right.json>\n\
     cargo xtask check <commit-preflight|developer-docs-impact|slice-review-impact> \
     <commit-message-file> [changed-paths-file] [branch]"
        .to_owned()
}

fn docs_accept_translation_usage() -> String {
    "usage: cargo xtask docs accept-translation <relative-page.md>".to_owned()
}

fn slice_contract_usage() -> String {
    "usage: cargo xtask slice-contract bind <slice-contract.json>".to_owned()
}

#[cfg(test)]
mod cli_tests {
    use super::{docs_accept_translation_usage, run};

    // 인자 없이 실행했을 때 서로 다른 입력 계약을 한 문장으로 섞지 않고,
    // 인자 없는 검사와 커밋 입력 검사를 각각 실행 가능한 형태로 안내한다.
    #[test]
    fn general_usage_separates_argument_free_and_impact_checks() {
        let error = run(Vec::<std::ffi::OsString>::new()).unwrap_err();

        assert_eq!(
            error,
            "usage:\n\
             cargo xtask docs accept-translation <relative-page.md>\n\
             cargo xtask slice-contract bind <slice-contract.json>\n\
             cargo xtask check test-explanations\n\
             cargo xtask check methexis-check-for-stage\n\
             cargo xtask check slice-scope [slice-contract.json]\n\
             cargo xtask check slice-parallel <left.json> <right.json>\n\
             cargo xtask check <commit-preflight|developer-docs-impact|slice-review-impact> \
             <commit-message-file> [changed-paths-file] [branch]"
        );
    }

    // test-explanations 뒤의 불필요한 인자는 조용히 무시하지 않고 해당 명령의
    // 정확한 사용법을 돌려줘 호출자가 잘못 구성한 훅을 바로 고칠 수 있게 한다.
    #[test]
    fn test_explanations_rejects_extra_arguments_with_specific_usage() {
        let error = run(["check", "test-explanations", "unexpected"].map(Into::into)).unwrap_err();

        assert_eq!(error, "usage: cargo xtask check test-explanations");
    }

    // 번역 승인 명령은 검토할 한 페이지를 반드시 요구하고 추가 인자를
    // 무시하지 않아, 호출자가 의도치 않게 여러 페이지를 승인하지 못하게 한다.
    #[test]
    fn docs_accept_translation_requires_exactly_one_page() {
        let missing = run(["docs", "accept-translation"].map(Into::into)).unwrap_err();
        let extra = run(["docs", "accept-translation", "README.md", "extra.md"].map(Into::into))
            .unwrap_err();

        assert_eq!(missing, docs_accept_translation_usage());
        assert_eq!(extra, docs_accept_translation_usage());
    }
}
