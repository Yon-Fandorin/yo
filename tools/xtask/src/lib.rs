mod activation_slice;
mod bounded_file;
mod docs_translation;
mod git;
mod impact;
mod review_delta;
mod review_packet;
mod slice_close;
mod slice_contract;
mod slice_worktree;
mod test_explanations;
mod validation_stage;

#[cfg(test)]
mod test_support;

use std::{ffi::OsString, path::PathBuf};

use impact::ImpactInput;

pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut arguments = arguments.into_iter();
    match (arguments.next().as_deref(), arguments.next().as_deref()) {
        (Some(command), Some(scope)) if command == "slice" && scope == "create-activation" => {
            let request = arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(activation_slice_usage)?;
            if arguments.next().is_some() {
                return Err(activation_slice_usage());
            }
            let repository = std::env::current_dir()
                .map_err(|error| format!("cannot locate the repository: {error}"))?;
            activation_slice::run(&repository, &request)
        },
        (Some(command), Some(scope)) if command == "slice" && scope == "review-packet" => {
            let request = arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(review_packet_usage)?;
            if arguments.next().is_some() {
                return Err(review_packet_usage());
            }
            let repository = std::env::current_dir()
                .map_err(|error| format!("cannot locate the repository: {error}"))?;
            review_packet::run(&repository, &request)
        },
        (Some(command), Some(scope)) if command == "slice" && scope == "review-delta" => {
            let request = arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(review_delta_usage)?;
            if arguments.next().is_some() {
                return Err(review_delta_usage());
            }
            let repository = std::env::current_dir()
                .map_err(|error| format!("cannot locate the repository: {error}"))?;
            review_delta::run(&repository, &request)
        },
        (Some(command), Some(scope)) if command == "slice" && scope == "close" => {
            let action = arguments
                .next()
                .ok_or_else(slice_close_usage)?
                .to_string_lossy()
                .into_owned();
            let value = arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(slice_close_usage)?;
            let output = arguments.next().map(PathBuf::from);
            if arguments.next().is_some() {
                return Err(slice_close_usage());
            }
            let repository = std::env::current_dir()
                .map_err(|error| format!("cannot locate the repository: {error}"))?;
            match action.as_str() {
                "plan" => {
                    let slice = value
                        .to_str()
                        .ok_or_else(|| "Slice name must be valid UTF-8".to_owned())?;
                    slice_close::plan(&repository, slice, output.as_deref())
                },
                "apply" if output.is_none() => slice_close::apply(&repository, &value),
                _ => Err(slice_close_usage()),
            }
        },
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
     cargo xtask slice create-activation <request.json>\n\
     cargo xtask slice review-packet <request.json>\n\
     cargo xtask slice review-delta <request.json>\n\
     cargo xtask slice close <plan SLICE [PLAN.json]|apply PLAN.json>\n\
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

fn activation_slice_usage() -> String {
    "usage: cargo xtask slice create-activation <request.json>".to_owned()
}

fn review_packet_usage() -> String {
    "usage: cargo xtask slice review-packet <request.json>".to_owned()
}

fn review_delta_usage() -> String {
    "usage: cargo xtask slice review-delta <request.json>".to_owned()
}

fn slice_close_usage() -> String {
    "usage: cargo xtask slice close <plan SLICE [PLAN.json]|apply PLAN.json>".to_owned()
}

fn docs_accept_translation_usage() -> String {
    "usage: cargo xtask docs accept-translation <relative-page.md>".to_owned()
}

fn slice_contract_usage() -> String {
    "usage: cargo xtask slice-contract bind <slice-contract.json>".to_owned()
}

#[cfg(test)]
mod cli_tests {
    use super::{
        activation_slice_usage, docs_accept_translation_usage, review_delta_usage,
        review_packet_usage, run, slice_close_usage,
    };

    // 인자 없이 실행했을 때 서로 다른 입력 계약을 한 문장으로 섞지 않고,
    // 인자 없는 검사와 커밋 입력 검사를 각각 실행 가능한 형태로 안내한다.
    #[test]
    fn general_usage_separates_argument_free_and_impact_checks() {
        let error = run(Vec::<std::ffi::OsString>::new()).unwrap_err();

        assert_eq!(
            error,
            "usage:\n\
             cargo xtask slice create-activation <request.json>\n\
             cargo xtask slice review-packet <request.json>\n\
             cargo xtask slice review-delta <request.json>\n\
             cargo xtask slice close <plan SLICE [PLAN.json]|apply PLAN.json>\n\
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

    // activation Slice 생성은 정확히 한 versioned request만 받아 누락되거나
    // 조용히 무시된 추가 입력으로 branch와 worktree를 만들지 않는다.
    #[test]
    fn activation_slice_requires_exactly_one_request() {
        let missing = run(["slice", "create-activation"].map(Into::into)).unwrap_err();
        let extra =
            run(["slice", "create-activation", "request.json", "extra.json"].map(Into::into))
                .unwrap_err();

        assert_eq!(missing, activation_slice_usage());
        assert_eq!(extra, activation_slice_usage());
    }

    // review packet 생성도 정확히 한 versioned request만 받아 입력 일부가
    // 누락되거나 추가 인자가 조용히 무시된 채 다른 review identity가 생기지 않는다.
    #[test]
    fn review_packet_requires_exactly_one_request() {
        let missing = run(["slice", "review-packet"].map(Into::into)).unwrap_err();
        let extra = run(["slice", "review-packet", "request.json", "extra.json"].map(Into::into))
            .unwrap_err();

        assert_eq!(missing, review_packet_usage());
        assert_eq!(extra, review_packet_usage());
    }

    // finding-resolution delta도 prior review identity와 disposition을 담은 정확히 한
    // versioned request만 받아 누락되거나 추가된 인자를 조용히 무시하지 않는다.
    #[test]
    fn review_delta_requires_exactly_one_request() {
        let missing = run(["slice", "review-delta"].map(Into::into)).unwrap_err();
        let extra = run(["slice", "review-delta", "request.json", "extra.json"].map(Into::into))
            .unwrap_err();

        assert_eq!(missing, review_delta_usage());
        assert_eq!(extra, review_delta_usage());
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

    // Slice close는 plan 또는 apply와 정확히 하나의 대상을 요구하여, 누락된
    // 정리 대상이나 조용히 무시되는 추가 인자가 파괴적 단계로 넘어가지 않는다.
    #[test]
    fn slice_close_rejects_incomplete_or_extra_arguments() {
        for arguments in [
            vec!["slice", "close"],
            vec!["slice", "close", "plan"],
            vec!["slice", "close", "apply"],
            vec!["slice", "close", "plan", "sample", "plan.json", "extra"],
            vec!["slice", "close", "apply", "plan.json", "extra"],
            vec!["slice", "close", "unknown", "sample"],
        ] {
            assert_eq!(
                run(arguments.into_iter().map(Into::into)).unwrap_err(),
                slice_close_usage()
            );
        }
    }
}
