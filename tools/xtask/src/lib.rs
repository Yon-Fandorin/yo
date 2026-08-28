mod activation_slice;
mod bounded_file;
mod cost_report;
mod docs_translation;
mod git;
mod impact;
mod review_continuation_preflight;
mod review_delivery;
mod review_delta;
mod review_egress;
mod review_packet;
mod review_prepare;
mod review_protocol;
mod review_result;
mod review_session;
mod review_target_admission;
mod slice_accept;
mod slice_close;
mod slice_contract;
mod slice_create;
mod slice_gate;
mod slice_status;
mod slice_worktree;
mod test_explanations;
mod validation_stage;
mod validation_summary;

#[cfg(test)]
mod test_support;

use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

use impact::ImpactInput;

pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut arguments = arguments.into_iter();
    let command = arguments.next();
    let scope = arguments.next();
    match (command.as_deref(), scope.as_deref()) {
        (Some(command), Some(target)) if command == "__accepted-commit-message-editor" => {
            run_accepted_commit_message_editor(target, &mut arguments)
        },
        (Some(command), Some(scope)) if command == "slice" => run_slice(scope, &mut arguments),
        (Some(command), Some(action)) if command == "docs" => run_docs(action, &mut arguments),
        (Some(command), Some(action)) if command == "slice-contract" => {
            run_slice_contract(action, &mut arguments)
        },
        (Some(command), Some(check)) if command == "check" => run_check(check, &mut arguments),
        _ => Err(general_usage()),
    }
}

fn run_slice(scope: &OsStr, arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    if scope == "create" {
        return run_slice_create(arguments);
    }
    if scope == "create-activation" {
        return run_activation_slice(arguments);
    }
    if scope == "review-packet" {
        return run_review_packet(arguments);
    }
    if scope == "review-prepare" {
        return run_review_prepare(arguments);
    }
    if scope == "review-delta" {
        return run_review_delta(arguments);
    }
    if scope == "review-deliver" {
        return run_review_delivery(arguments);
    }
    if scope == "review-continuation-preflight" {
        return run_review_continuation_preflight(arguments);
    }
    if scope == "cost-report" {
        return run_cost_report(arguments);
    }
    if scope == "review-egress" {
        return run_review_egress(arguments);
    }
    if scope == "review-target-admission" {
        return run_review_target_admission(arguments);
    }
    if scope == "close" {
        return run_slice_close(arguments);
    }
    if scope == "gate" {
        return run_slice_gate(arguments);
    }
    if scope == "commit" {
        return run_slice_commit(arguments);
    }
    if scope == "accept" {
        return run_slice_accept(arguments);
    }
    if scope == "status" {
        return run_slice_status(arguments);
    }
    Err(general_usage())
}

fn run_slice_create(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let contract = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(slice_create_usage)?;
    if arguments.next().is_some() {
        return Err(slice_create_usage());
    }
    let repository = current_repository()?;
    slice_create::run(&repository, &contract)
}

fn run_cost_report(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(cost_report_usage)?;
    let output = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(cost_report_usage)?;
    if arguments.next().is_some() {
        return Err(cost_report_usage());
    }
    let repository = current_repository()?;
    cost_report::run(&repository, &request, &output)
}

fn run_review_prepare(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(review_prepare_usage)?;
    if arguments.next().is_some() {
        return Err(review_prepare_usage());
    }
    let repository = current_repository()?;
    review_prepare::run(&repository, &request)
}

fn run_slice_accept(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let first = arguments.next().ok_or_else(slice_accept_usage)?;
    if first == "prepare" {
        let request = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_accept_usage)?;
        if arguments.next().is_some() {
            return Err(slice_accept_usage());
        }
        let repository = current_repository()?;
        return slice_accept::prepare(&repository, &request);
    }
    let request = PathBuf::from(first);
    if arguments.next().is_some() {
        return Err(slice_accept_usage());
    }
    let repository = current_repository()?;
    slice_accept::accept(&repository, &request)
}

fn run_slice_status(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let slice = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(slice_status_usage)?;
    if arguments.next().is_some() {
        return Err(slice_status_usage());
    }
    let repository = current_repository()?;
    slice_status::run(&repository, &slice)
}

fn run_slice_gate(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let first = arguments.next().ok_or_else(slice_gate_usage)?;
    if first == "prepare" {
        let request = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_gate_usage)?;
        let output = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_gate_usage)?;
        if arguments.next().is_some() {
            return Err(slice_gate_usage());
        }
        let repository = current_repository()?;
        return slice_gate::prepare_request(&repository, &request, &output);
    }
    let request = PathBuf::from(first);
    if arguments.next().is_some() {
        return Err(slice_gate_usage());
    }
    let repository = current_repository()?;
    slice_gate::run(&repository, &request)
}

fn run_slice_commit(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let first = arguments.next().ok_or_else(slice_commit_usage)?;
    if first == "prepare" {
        let gate_request = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_commit_usage)?;
        let message_source = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_commit_usage)?;
        let output = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_commit_usage)?;
        if arguments.next().is_some() {
            return Err(slice_commit_usage());
        }
        let repository = current_repository()?;
        return slice_accept::prepare_commit_message(
            &repository,
            &gate_request,
            &message_source,
            &output,
        );
    }
    let message = PathBuf::from(first);
    if arguments.next().is_some() {
        return Err(slice_commit_usage());
    }
    let input = ImpactInput::load(message.clone(), None, None, true)?;
    impact::preflight::check(&input)?;
    impact::review_coverage::create_accepted_commit(&input.repository, &message)
}

fn run_accepted_commit_message_editor(
    target: &OsStr,
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    if arguments.next().is_some() {
        return Err("invalid internal accepted-commit editor invocation".to_owned());
    }
    impact::review_coverage::copy_accepted_commit_message(&PathBuf::from(target))
}

fn run_activation_slice(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(activation_slice_usage)?;
    if arguments.next().is_some() {
        return Err(activation_slice_usage());
    }
    let repository = current_repository()?;
    activation_slice::run(&repository, &request)
}

fn run_review_packet(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let first = arguments.next().ok_or_else(review_packet_usage)?;
    let (mode, request) = match first.to_str() {
        Some("--check-readiness") => (
            ReviewPacketMode::CheckReadiness,
            arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(review_packet_usage)?,
        ),
        Some("--preflight") => (
            ReviewPacketMode::Preflight,
            arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(review_packet_usage)?,
        ),
        _ => (ReviewPacketMode::Publish, PathBuf::from(first)),
    };
    if arguments.next().is_some() {
        return Err(review_packet_usage());
    }
    let repository = current_repository()?;
    match mode {
        ReviewPacketMode::CheckReadiness => {
            review_packet::check_readiness(&repository, &request, &mut std::io::stdout().lock())
        },
        ReviewPacketMode::Preflight => {
            review_packet::preflight(&repository, &request, &mut std::io::stdout().lock())
        },
        ReviewPacketMode::Publish => review_packet::run(&repository, &request),
    }
}

enum ReviewPacketMode {
    CheckReadiness,
    Preflight,
    Publish,
}

fn run_review_delta(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(review_delta_usage)?;
    if arguments.next().is_some() {
        return Err(review_delta_usage());
    }
    let repository = current_repository()?;
    review_delta::run(&repository, &request)
}

fn run_review_egress(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(review_egress_usage)?;
    if arguments.next().is_some() {
        return Err(review_egress_usage());
    }
    let repository = current_repository()?;
    review_egress::run(&repository, &request)
}

fn run_review_delivery(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let first = arguments.next().ok_or_else(review_delivery_usage)?;
    let (finalize, request) = if first == "finalize" {
        (
            true,
            arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(review_delivery_usage)?,
        )
    } else {
        (false, PathBuf::from(first))
    };
    if arguments.next().is_some() {
        return Err(review_delivery_usage());
    }
    let repository = current_repository()?;
    if finalize {
        review_delivery::finalize(&repository, &request)
    } else {
        review_delivery::run(&repository, &request)
    }
}

fn run_review_target_admission(
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(review_target_admission_usage)?;
    if arguments.next().is_some() {
        return Err(review_target_admission_usage());
    }
    review_target_admission::run(&request)
}

fn run_review_continuation_preflight(
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    let request = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(review_continuation_preflight_usage)?;
    if arguments.next().is_some() {
        return Err(review_continuation_preflight_usage());
    }
    let repository = current_repository()?;
    review_continuation_preflight::run(&repository, &request)
}

fn run_slice_close(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let action = arguments
        .next()
        .ok_or_else(slice_close_usage)?
        .to_string_lossy()
        .into_owned();
    if action == "prepare" {
        let request = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(slice_close_usage)?;
        if arguments.next().is_some() {
            return Err(slice_close_usage());
        }
        let repository = current_repository()?;
        return slice_close::prepare_metrics(&repository, &request);
    }
    let value = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(slice_close_usage)?;
    let output = arguments.next().map(PathBuf::from);
    if arguments.next().is_some() {
        return Err(slice_close_usage());
    }
    let repository = current_repository()?;
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
}

fn run_docs(action: &OsStr, arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    if action == "accept-translation" {
        return run_docs_accept_translation(arguments);
    }
    Err(general_usage())
}

fn run_docs_accept_translation(
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    let page = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(docs_accept_translation_usage)?;
    if arguments.next().is_some() {
        return Err(docs_accept_translation_usage());
    }
    let repository = current_repository()?;
    docs_translation::accept(&repository, &page)
}

fn run_slice_contract(
    action: &OsStr,
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    if action == "bind" {
        return run_slice_contract_bind(arguments);
    }
    Err(general_usage())
}

fn run_slice_contract_bind(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let contract = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(slice_contract_usage)?;
    if arguments.next().is_some() {
        return Err(slice_contract_usage());
    }
    let repository = current_repository()?;
    slice_contract::bind(&repository, &contract)
}

fn run_check(check: &OsStr, arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let check = check.to_string_lossy();
    match check.as_ref() {
        "test-explanations" => run_test_explanations_check(arguments),
        "slice-scope" => run_slice_scope_check(arguments),
        "slice-parallel" => run_slice_parallel_check(arguments),
        "wave-assembly" => run_wave_assembly_check(arguments),
        "methexis-check-for-stage" => run_methexis_check_for_stage(arguments),
        "review-coverage-operation" => run_review_coverage_operation_check(arguments),
        "commit-preflight" | "developer-docs-impact" | "slice-review-impact" => {
            run_impact_check(arguments, check.as_ref())
        },
        _ => Err(usage(check.as_ref())),
    }
}

fn run_test_explanations_check(
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    if arguments.next().is_some() {
        return Err(usage("test-explanations"));
    }
    let repository = current_repository()?;
    test_explanations::check(&repository)
}

fn run_slice_scope_check(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let contract = arguments.next().map(PathBuf::from);
    if arguments.next().is_some() {
        return Err(usage("slice-scope"));
    }
    let repository = current_repository()?;
    match contract {
        Some(contract) => slice_contract::check_scope(&repository, &contract),
        None => slice_contract::check_bound_scope(&repository),
    }
}

fn run_slice_parallel_check(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let left = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage("slice-parallel"))?;
    let right = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage("slice-parallel"))?;
    if arguments.next().is_some() {
        return Err(usage("slice-parallel"));
    }
    let repository = current_repository()?;
    slice_contract::check_parallel(&repository, &left, &right)
}

fn run_wave_assembly_check(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let boundary = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage("wave-assembly"))?;
    let components = arguments.map(PathBuf::from).collect::<Vec<_>>();
    if components.is_empty() {
        return Err(usage("wave-assembly"));
    }
    let repository = current_repository()?;
    slice_contract::check_wave_assembly(&repository, &boundary, &components)
}

fn run_methexis_check_for_stage(
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    if arguments.next().is_some() {
        return Err(usage("methexis-check-for-stage"));
    }
    let repository = current_repository()?;
    validation_stage::run_methexis_check(&repository)
}

fn run_review_coverage_operation_check(
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    let _message = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage("review-coverage-operation"))?;
    let source = arguments
        .next()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "prepare-commit-msg source must be valid UTF-8".to_owned())
        })
        .transpose()?;
    let commit = arguments
        .next()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "prepare-commit-msg commit must be valid UTF-8".to_owned())
        })
        .transpose()?;
    if arguments.next().is_some() {
        return Err(usage("review-coverage-operation"));
    }
    let repository = current_repository()?;
    impact::review_coverage::check_prepare_commit_message(
        &repository,
        source.as_deref(),
        commit.as_deref(),
    )
}

fn run_impact_check(
    arguments: &mut impl Iterator<Item = OsString>,
    check: &str,
) -> Result<(), String> {
    let head_fallback = matches!(check, "commit-preflight" | "slice-review-impact");
    let message = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage(check))?;
    let changed_paths = arguments.next().map(PathBuf::from);
    let branch = arguments
        .next()
        .map(|value| value.to_string_lossy().into_owned());
    if arguments.next().is_some() {
        return Err(usage(check));
    }
    let input = ImpactInput::load(message, changed_paths, branch, head_fallback)?;
    match check {
        "commit-preflight" => impact::preflight::check(&input),
        "developer-docs-impact" => impact::developer_docs::check(&input),
        "slice-review-impact" => impact::slice_review::check(&input),
        _ => unreachable!("the check name was validated before loading input"),
    }
}

fn current_repository() -> Result<PathBuf, String> {
    std::env::current_dir().map_err(|error| format!("cannot locate the repository: {error}"))
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
        "wave-assembly" => {
            return "usage: cargo xtask check wave-assembly <boundary.json> <component.json>..."
                .to_owned();
        },
        "review-coverage-operation" => {
            return "usage: cargo xtask check review-coverage-operation \
                    <commit-message-file> [source] [commit]"
                .to_owned();
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
     cargo xtask slice create <slice-contract.json>\n\
     cargo xtask slice create-activation <request.json>\n\
     cargo xtask slice review-packet [--check-readiness|--preflight] <request.json>\n\
     cargo xtask slice review-prepare <request.json>\n\
     cargo xtask slice review-delta <request.json>\n\
     cargo xtask slice review-egress <request.json>\n\
     cargo xtask slice review-target-admission <request.json>\n\
     cargo xtask slice review-deliver <request.json|finalize FINALIZE.json>\n\
     cargo xtask slice review-continuation-preflight <request.json>\n\
     cargo xtask slice cost-report <request.json> <output.json>\n\
     cargo xtask slice gate <request.json>\n\
     cargo xtask slice gate prepare <prepare.json> <gate.json>\n\
     cargo xtask slice close <prepare REQUEST.json|plan SLICE [PLAN.json]|apply PLAN.json>\n\
     cargo xtask slice commit <commit-message-file|prepare GATE.json MESSAGE-SOURCE MESSAGE-OUT>\n\
     cargo xtask slice accept <request.json|prepare PREPARE.json>\n\
     cargo xtask slice status <slice>\n\
     cargo xtask docs accept-translation <relative-page.md>\n\
     cargo xtask slice-contract bind <slice-contract.json>\n\
     cargo xtask check test-explanations\n\
     cargo xtask check methexis-check-for-stage\n\
     cargo xtask check slice-scope [slice-contract.json]\n\
     cargo xtask check slice-parallel <left.json> <right.json>\n\
     cargo xtask check wave-assembly <boundary.json> <component.json>...\n\
     cargo xtask check review-coverage-operation <commit-message-file> [source] [commit]\n\
     cargo xtask check <commit-preflight|developer-docs-impact|slice-review-impact> \
     <commit-message-file> [changed-paths-file] [branch]"
        .to_owned()
}

fn slice_create_usage() -> String {
    "usage: cargo xtask slice create <slice-contract.json>".to_owned()
}

fn activation_slice_usage() -> String {
    "usage: cargo xtask slice create-activation <request.json>".to_owned()
}

fn review_packet_usage() -> String {
    "usage: cargo xtask slice review-packet [--check-readiness|--preflight] <request.json>"
        .to_owned()
}

fn review_prepare_usage() -> String {
    "usage: cargo xtask slice review-prepare <request.json>".to_owned()
}

fn review_delta_usage() -> String {
    "usage: cargo xtask slice review-delta <request.json>".to_owned()
}

fn review_egress_usage() -> String {
    "usage: cargo xtask slice review-egress <request.json>".to_owned()
}

fn review_delivery_usage() -> String {
    "usage: cargo xtask slice review-deliver <request.json|finalize FINALIZE.json>".to_owned()
}

fn review_target_admission_usage() -> String {
    "usage: cargo xtask slice review-target-admission <request.json>".to_owned()
}

fn review_continuation_preflight_usage() -> String {
    "usage: cargo xtask slice review-continuation-preflight <request.json>".to_owned()
}

fn cost_report_usage() -> String {
    "usage: cargo xtask slice cost-report <request.json> <output.json>".to_owned()
}

fn slice_close_usage() -> String {
    "usage: cargo xtask slice close <prepare REQUEST.json|plan SLICE [PLAN.json]|apply PLAN.json>"
        .to_owned()
}

fn slice_gate_usage() -> String {
    "usage: cargo xtask slice gate <request.json>\n       cargo xtask slice gate prepare <prepare.json> <gate.json>".to_owned()
}

fn slice_commit_usage() -> String {
    "usage: cargo xtask slice commit <commit-message-file>\n       cargo xtask slice commit prepare <gate.json> <message-source> <message-out>".to_owned()
}

fn slice_status_usage() -> String {
    "usage: cargo xtask slice status <slice>".to_owned()
}

fn slice_accept_usage() -> String {
    "usage: cargo xtask slice accept <request.json>\n       cargo xtask slice accept prepare <prepare.json>".to_owned()
}

fn docs_accept_translation_usage() -> String {
    "usage: cargo xtask docs accept-translation <relative-page.md>".to_owned()
}

fn slice_contract_usage() -> String {
    "usage: cargo xtask slice-contract bind <slice-contract.json>".to_owned()
}

#[cfg(test)]
mod cli_tests {
    use std::{cell::Cell, ffi::OsString};

    use super::{
        activation_slice_usage, cost_report_usage, docs_accept_translation_usage,
        review_continuation_preflight_usage, review_delivery_usage, review_delta_usage,
        review_egress_usage, review_packet_usage, review_prepare_usage,
        review_target_admission_usage, run, slice_accept_usage, slice_close_usage,
        slice_commit_usage, slice_create_usage, slice_gate_usage, slice_status_usage,
    };

    // 인자 없이 실행했을 때 서로 다른 입력 계약을 한 문장으로 섞지 않고,
    // 인자 없는 검사와 커밋 입력 검사를 각각 실행 가능한 형태로 안내한다.
    #[test]
    fn general_usage_separates_argument_free_and_impact_checks() {
        let error = run(Vec::<std::ffi::OsString>::new()).unwrap_err();

        assert_eq!(
            error,
            "usage:\n\
             cargo xtask slice create <slice-contract.json>\n\
             cargo xtask slice create-activation <request.json>\n\
             cargo xtask slice review-packet [--check-readiness|--preflight] <request.json>\n\
             cargo xtask slice review-prepare <request.json>\n\
             cargo xtask slice review-delta <request.json>\n\
             cargo xtask slice review-egress <request.json>\n\
             cargo xtask slice review-target-admission <request.json>\n\
             cargo xtask slice review-deliver <request.json|finalize FINALIZE.json>\n\
             cargo xtask slice review-continuation-preflight <request.json>\n\
             cargo xtask slice cost-report <request.json> <output.json>\n\
             cargo xtask slice gate <request.json>\n\
             cargo xtask slice gate prepare <prepare.json> <gate.json>\n\
             cargo xtask slice close <prepare REQUEST.json|plan SLICE [PLAN.json]|apply PLAN.json>\n\
             cargo xtask slice commit <commit-message-file|prepare GATE.json MESSAGE-SOURCE MESSAGE-OUT>\n\
             cargo xtask slice accept <request.json|prepare PREPARE.json>\n\
             cargo xtask slice status <slice>\n\
             cargo xtask docs accept-translation <relative-page.md>\n\
             cargo xtask slice-contract bind <slice-contract.json>\n\
             cargo xtask check test-explanations\n\
             cargo xtask check methexis-check-for-stage\n\
             cargo xtask check slice-scope [slice-contract.json]\n\
             cargo xtask check slice-parallel <left.json> <right.json>\n\
             cargo xtask check wave-assembly <boundary.json> <component.json>...\n\
             cargo xtask check review-coverage-operation \
             <commit-message-file> [source] [commit]\n\
             cargo xtask check <commit-preflight|developer-docs-impact|slice-review-impact> \
             <commit-message-file> [changed-paths-file] [branch]"
        );
    }

    // 일반 Slice bootstrap도 정확히 한 immutable 계약만 받아 누락되거나 추가된
    // positional input이 별도 branch identity로 해석되지 않게 합니다.
    #[test]
    fn slice_create_requires_exactly_one_contract() {
        let missing = run(["slice", "create"].map(Into::into)).unwrap_err();
        let extra = run(["slice", "create", "slice-contract.json", "extra.json"].map(Into::into))
            .unwrap_err();

        assert_eq!(missing, slice_create_usage());
        assert_eq!(extra, slice_create_usage());
    }

    // 최상위 명령 분배는 첫 번째 명령이 없거나 알 수 없는 경우에도 기존 구현처럼 비교할 두 값을
    // 모두 미리 읽어야 하므로, 호출 횟수를 관찰할 수 있는 반복자에서도 두 번 읽는 계약을 유지한다.
    #[test]
    fn general_dispatch_prefetches_command_and_scope() {
        let empty_calls = Cell::new(0);
        let empty = std::iter::from_fn(|| {
            empty_calls.set(empty_calls.get() + 1);
            None::<OsString>
        });
        assert!(run(empty).is_err());
        assert_eq!(empty_calls.get(), 2);

        let unknown_calls = Cell::new(0);
        let mut values = ["unknown", "scope"].into_iter().map(OsString::from);
        let unknown = std::iter::from_fn(|| {
            unknown_calls.set(unknown_calls.get() + 1);
            values.next()
        });
        assert!(run(unknown).is_err());
        assert_eq!(unknown_calls.get(), 2);
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

    // 비용 owner report는 source request와 새 output 경로를 모두 요구하며 추가 인자를
    // 무시한 채 다른 artifact를 만들지 않는다.
    #[test]
    fn cost_report_requires_request_and_output() {
        let missing = run(["slice", "cost-report", "request.json"].map(Into::into)).unwrap_err();
        let extra = run([
            "slice",
            "cost-report",
            "request.json",
            "output.json",
            "extra.json",
        ]
        .map(Into::into))
        .unwrap_err();

        assert_eq!(missing, cost_report_usage());
        assert_eq!(extra, cost_report_usage());
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

    // 통합 review preparation도 한 versioned request만 받아 semantic 입력과 target을
    // 추가 argv로 교체하거나 두 번째 준비 경로를 암묵적으로 만들지 않습니다.
    #[test]
    fn review_prepare_requires_exactly_one_request() {
        let missing = run(["slice", "review-prepare"].map(Into::into)).unwrap_err();
        let extra = run(["slice", "review-prepare", "request.json", "extra.json"].map(Into::into))
            .unwrap_err();

        assert_eq!(missing, review_prepare_usage());
        assert_eq!(extra, review_prepare_usage());
    }

    // preflight도 publication과 같은 하나의 versioned request를 요구하여, request가
    // 빠지거나 추가 입력이 있는 호출을 준비 완료로 오인하지 않는다.
    #[test]
    fn review_packet_preflight_requires_exactly_one_request() {
        let missing = run(["slice", "review-packet", "--preflight"].map(Into::into)).unwrap_err();
        let extra = run([
            "slice",
            "review-packet",
            "--preflight",
            "request.json",
            "extra.json",
        ]
        .map(Into::into))
        .unwrap_err();

        assert_eq!(missing, review_packet_usage());
        assert_eq!(extra, review_packet_usage());
    }

    // readiness도 publication과 같은 request 하나만 받아, 빠진 입력이나 서로 겹친 mode
    // flag를 준비 완료로 처리하지 않는다.
    #[test]
    fn review_packet_readiness_requires_exactly_one_request() {
        let missing =
            run(["slice", "review-packet", "--check-readiness"].map(Into::into)).unwrap_err();
        let extra = run([
            "slice",
            "review-packet",
            "--check-readiness",
            "request.json",
            "extra.json",
        ]
        .map(Into::into))
        .unwrap_err();
        let overlapping = run([
            "slice",
            "review-packet",
            "--check-readiness",
            "--preflight",
            "request.json",
        ]
        .map(Into::into))
        .unwrap_err();

        assert_eq!(missing, review_packet_usage());
        assert_eq!(extra, review_packet_usage());
        assert_eq!(overlapping, review_packet_usage());
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

    // external review egress preflight도 packet과 standing authorization을 결속한 정확히
    // 한 request만 받아 추가 입력으로 route나 권한을 넓히지 않는다.
    #[test]
    fn review_egress_requires_exactly_one_request() {
        let missing = run(["slice", "review-egress"].map(Into::into)).unwrap_err();
        let extra = run(["slice", "review-egress", "request.json", "extra.json"].map(Into::into))
            .unwrap_err();

        assert_eq!(missing, review_egress_usage());
        assert_eq!(extra, review_egress_usage());
    }

    // target admission도 정확히 한 versioned request만 받아 호출자가 argv로 다른
    // target이나 상태 경로를 덧붙이지 못하게 합니다.
    #[test]
    fn review_target_admission_requires_exactly_one_request() {
        let missing = run(["slice", "review-target-admission"].map(Into::into)).unwrap_err();
        let extra = run([
            "slice",
            "review-target-admission",
            "request.json",
            "extra.json",
        ]
        .map(Into::into))
        .unwrap_err();

        assert_eq!(missing, review_target_admission_usage());
        assert_eq!(extra, review_target_admission_usage());
    }

    // 실제 외부 effect를 소유하는 review-deliver도 versioned request 하나만 받아
    // 추가 argv가 route, packet, retry 또는 output 경계를 넓히지 못하게 한다.
    #[test]
    fn review_delivery_requires_exactly_one_request() {
        let missing = run(["slice", "review-deliver"].map(Into::into)).unwrap_err();
        let extra = run(["slice", "review-deliver", "request.json", "extra.json"].map(Into::into))
            .unwrap_err();

        assert_eq!(missing, review_delivery_usage());
        assert_eq!(extra, review_delivery_usage());
        let finalize_missing =
            run(["slice", "review-deliver", "finalize"].map(Into::into)).unwrap_err();
        assert_eq!(finalize_missing, review_delivery_usage());
    }

    // status와 accept도 Slice 이름 또는 versioned request 하나만 받아 compact 관측과
    // mutation 경계가 여분 인자로 달라지지 않으며, prepare 역시 입력 하나만 받습니다.
    #[test]
    fn status_and_accept_require_one_input() {
        for (scope, usage) in [
            ("status", slice_status_usage()),
            ("accept", slice_accept_usage()),
        ] {
            assert_eq!(run(["slice", scope].map(Into::into)).unwrap_err(), usage);
            assert_eq!(
                run(["slice", scope, "input", "extra"].map(Into::into)).unwrap_err(),
                usage
            );
        }
        assert_eq!(
            run(["slice", "accept", "prepare"].map(Into::into)).unwrap_err(),
            slice_accept_usage()
        );
        assert_eq!(
            run(["slice", "accept", "prepare", "input", "extra"].map(Into::into)).unwrap_err(),
            slice_accept_usage()
        );
    }

    // finding-resolution preflight도 egress와 Session root를 담은 closed request 하나만
    // 받아 추가 argv가 terminal input이나 resume 대상을 넓히지 못하게 합니다.
    #[test]
    fn review_continuation_preflight_requires_exactly_one_request() {
        let missing = run(["slice", "review-continuation-preflight"].map(Into::into)).unwrap_err();
        let extra = run([
            "slice",
            "review-continuation-preflight",
            "request.json",
            "extra.json",
        ]
        .map(Into::into))
        .unwrap_err();

        assert_eq!(missing, review_continuation_preflight_usage());
        assert_eq!(extra, review_continuation_preflight_usage());
    }

    // Slice gate는 한 후보에 결속된 단일 request만 받아 서로 다른 후보의 증거가
    // 추가 인자로 섞이거나 request 없는 기본 동작으로 승인되지 않게 한다.
    #[test]
    fn slice_gate_requires_exactly_one_request() {
        let missing = run(["slice", "gate"].map(Into::into)).unwrap_err();
        let extra =
            run(["slice", "gate", "request.json", "extra.json"].map(Into::into)).unwrap_err();

        assert_eq!(missing, slice_gate_usage());
        assert_eq!(extra, slice_gate_usage());
    }

    // gate preparation은 compact source request와 별도의 immutable gate output 경로를
    // 정확히 하나씩 요구하여 누락·덮어쓰기·추가 입력을 명령 경계에서 거부한다.
    #[test]
    fn slice_gate_prepare_requires_request_and_output() {
        let missing_both = run(["slice", "gate", "prepare"].map(Into::into)).unwrap_err();
        let missing_output =
            run(["slice", "gate", "prepare", "prepare.json"].map(Into::into)).unwrap_err();
        let extra = run([
            "slice",
            "gate",
            "prepare",
            "prepare.json",
            "gate.json",
            "extra.json",
        ]
        .map(Into::into))
        .unwrap_err();

        assert_eq!(missing_both, slice_gate_usage());
        assert_eq!(missing_output, slice_gate_usage());
        assert_eq!(extra, slice_gate_usage());
    }

    // test-explanations 뒤의 불필요한 인자는 조용히 무시하지 않고 해당 명령의
    // 정확한 사용법을 돌려줘 호출자가 잘못 구성한 훅을 바로 고칠 수 있게 한다.
    #[test]
    fn test_explanations_rejects_extra_arguments_with_specific_usage() {
        let error = run(["check", "test-explanations", "unexpected"].map(Into::into)).unwrap_err();

        assert_eq!(error, "usage: cargo xtask check test-explanations");
    }

    // prepare-commit-msg 경계는 Git이 전달하는 메시지 파일과 선택적 source/commit만
    // 받아, 누락되거나 추가된 hook 인자를 다른 커밋 동작으로 오인하지 않는다.
    #[test]
    fn review_coverage_operation_requires_the_prepare_commit_message_shape() {
        let expected = super::usage("review-coverage-operation");
        let missing = run(["check", "review-coverage-operation"].map(Into::into)).unwrap_err();
        let extra = run([
            "check",
            "review-coverage-operation",
            "message",
            "commit",
            "0123456789abcdef0123456789abcdef01234567",
            "extra",
        ]
        .map(Into::into))
        .unwrap_err();

        assert_eq!(missing, expected);
        assert_eq!(extra, expected);
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
            vec!["slice", "close", "prepare"],
            vec!["slice", "close", "prepare", "request.json", "extra"],
            vec!["slice", "close", "unknown", "sample"],
        ] {
            assert_eq!(
                run(arguments.into_iter().map(Into::into)).unwrap_err(),
                slice_close_usage()
            );
        }
    }

    // accepted commit과 prepare는 각자의 exact 입력 개수만 받아, 누락되거나 추가된
    // 경로로 다른 게이트 또는 메시지를 소비하지 않는다.
    #[test]
    fn slice_commit_requires_exactly_one_prepared_message() {
        let missing = run(["slice", "commit"].map(Into::into)).unwrap_err();
        let extra = run(["slice", "commit", "message", "extra"].map(Into::into)).unwrap_err();
        let prepare_missing =
            run(["slice", "commit", "prepare", "gate", "source"].map(Into::into)).unwrap_err();
        let prepare_extra = run([
            "slice", "commit", "prepare", "gate", "source", "out", "extra",
        ]
        .map(Into::into))
        .unwrap_err();

        assert_eq!(missing, slice_commit_usage());
        assert_eq!(extra, slice_commit_usage());
        assert_eq!(prepare_missing, slice_commit_usage());
        assert_eq!(prepare_extra, slice_commit_usage());
    }
}
