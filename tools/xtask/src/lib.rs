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
                let contract = arguments
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| usage(check.as_ref()))?;
                if arguments.next().is_some() {
                    return Err(usage(check.as_ref()));
                }
                let repository = std::env::current_dir()
                    .map_err(|error| format!("cannot locate the repository: {error}"))?;
                return slice_contract::check_scope(&repository, &contract);
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
            if check == "methexis-tests-for-stage" {
                if arguments.next().is_some() {
                    return Err(usage(check.as_ref()));
                }
                let repository = std::env::current_dir()
                    .map_err(|error| format!("cannot locate the repository: {error}"))?;
                return validation_stage::run_methexis_tests(&repository);
            }

            let head_fallback = check == "slice-review-impact";
            if !matches!(
                check.as_ref(),
                "developer-docs-impact" | "slice-review-impact"
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
        "test-explanations" | "methexis-tests-for-stage" => {
            return format!("usage: cargo xtask check {check}");
        },
        "slice-scope" => {
            return "usage: cargo xtask check slice-scope <slice-contract.json>".to_owned();
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
     cargo xtask check test-explanations\n\
     cargo xtask check methexis-tests-for-stage\n\
     cargo xtask check slice-scope <slice-contract.json>\n\
     cargo xtask check slice-parallel <left.json> <right.json>\n\
     cargo xtask check <developer-docs-impact|slice-review-impact> \
     <commit-message-file> [changed-paths-file] [branch]"
        .to_owned()
}

#[cfg(test)]
mod cli_tests {
    use super::run;

    // 인자 없이 실행했을 때 서로 다른 입력 계약을 한 문장으로 섞지 않고,
    // 인자 없는 검사와 커밋 입력 검사를 각각 실행 가능한 형태로 안내한다.
    #[test]
    fn general_usage_separates_argument_free_and_impact_checks() {
        let error = run(Vec::<std::ffi::OsString>::new()).unwrap_err();

        assert_eq!(
            error,
            "usage:\n\
             cargo xtask check test-explanations\n\
             cargo xtask check methexis-tests-for-stage\n\
             cargo xtask check slice-scope <slice-contract.json>\n\
             cargo xtask check slice-parallel <left.json> <right.json>\n\
             cargo xtask check <developer-docs-impact|slice-review-impact> \
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
}
