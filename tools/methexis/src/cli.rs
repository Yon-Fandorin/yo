use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, Write},
    path::Path,
    process::ExitCode,
};

use serde::Serialize;

use crate::{
    CheckClass,
    author::AuthorService,
    check_repository_selected,
    checkpoint::{CheckpointService, StagedTransition},
    context::ContextService,
    review::ReviewService,
};

const HELP: &str = concat!(
    "methexis ",
    env!("CARGO_PKG_VERSION"),
    "
Methexis SOT Pilot

USAGE:
    methexis [--help | --version]
    methexis capabilities
    methexis check [--only <class>[,<class>...]]... [--summary] [--unit <id>]
    methexis check --staged-activation
    methexis author-revision <request.json>
    methexis project-review <request.json>
    methexis build-review <request.json>
    methexis prepare-approval <manifest.json> --reviewer <owner-id> [--replace-current]
    methexis prepare-approval --canonical <knowledge-id> --revision <sha256:revision> --reviewer <owner-id> [--replace-current]
    methexis approve <request.json>
    methexis prepare-checkpoint
    methexis create-checkpoint <request.json>
    methexis prepare-activation <create-output.json>
    methexis propose-activation <request.json>
    methexis refresh-context-manifests <activation-request.json>
    methexis resolve-context <request.json>
    methexis resolve-activation-review-context <activation-request.json> <context-request.json>
    methexis verify-context-build <request.json> <sha256:BuildId>

COMMANDS:
    capabilities      Report complete supported workflow profiles
    check             Validate current SOT integrity or one exact staged activation
    author-revision   Author a derived unit revision as tracked Draft proposals
    project-review    Write a tracked Korean review Projection
    build-review      Build a local human-review packet
    prepare-approval  Emit a Projection or canonical-basis approval request
    approve           Record a human-authorized approval proposal
    prepare-checkpoint Emit a Checkpoint request from the active roots
    create-checkpoint Create an immutable trusted-revision Checkpoint proposal
    prepare-activation Emit an activation request from create-checkpoint output
    propose-activation Propose the active Checkpoint with compare-and-swap
    refresh-context-manifests Refresh registered manifests for an activation proposal
    resolve-context    Build or reuse deterministic token-bounded agent context
    resolve-activation-review-context Build review-only context from one activation proposal
    verify-context-build Independently reproduce and verify one managed ContextBuild

Run commands from the repository root. Mutations remain Draft proposals until
trusted integration. Check derives approval and active/degraded eligibility
from local develop, then uses current Source observations only to demote it.
",
);

const UNSUPPORTED_COMMAND: &str = "\
{\"schema\":\"methexis.error/v1alpha1\",\"ok\":false,\"error\":{\"code\":\"unsupported_command\",\"affected_ids\":[],\"next_actions\":[\"methexis --help\"]}}
";

#[derive(Serialize)]
struct Capabilities {
    schema: &'static str,
    capabilities: [&'static str; 2],
}

/// Runs the current Methexis command surface against explicit streams.
pub fn run(
    args: impl IntoIterator<Item = OsString>,
    mut stdout: impl Write,
    mut stderr: impl Write,
) -> io::Result<ExitCode> {
    let args = args.into_iter().collect::<Vec<_>>();

    if let Some(result) = run_bootstrap(&args, &mut stdout) {
        return result;
    }

    run_command(&args, &mut stdout, &mut stderr)
}

fn run_bootstrap(args: &[OsString], stdout: &mut impl Write) -> Option<io::Result<ExitCode>> {
    match args {
        [] => Some(write_text(stdout, HELP, ExitCode::SUCCESS)),
        [arg] if arg == OsStr::new("--help") || arg == OsStr::new("-h") => {
            Some(write_text(stdout, HELP, ExitCode::SUCCESS))
        },
        [arg] if arg == OsStr::new("--version") || arg == OsStr::new("-V") => Some(
            writeln!(stdout, "methexis {}", env!("CARGO_PKG_VERSION")).map(|()| ExitCode::SUCCESS),
        ),
        [command] if command == OsStr::new("capabilities") => Some(write_json(
            stdout,
            &Capabilities {
                schema: "methexis.capabilities/v1",
                capabilities: [
                    "canonical-approval-on-demand-projection/v1",
                    "semantic-first-ko-on-demand/v1",
                ],
            },
            ExitCode::SUCCESS,
        )),
        _ => None,
    }
}

fn run_command(
    args: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    if args.first().is_some_and(|arg| arg == OsStr::new("check")) {
        return run_check(&args[1..], stdout, stderr);
    }

    if args
        .first()
        .is_some_and(|arg| arg == OsStr::new("prepare-approval"))
    {
        return run_prepare_approval(&args[1..], stdout, stderr);
    }

    match args {
        [command] if command == OsStr::new("prepare-checkpoint") => {
            run_prepare_checkpoint(stdout, stderr)
        },
        [command, request] if command == OsStr::new("author-revision") => {
            run_author_operation(request, stdout, stderr)
        },
        [command, request] if command == OsStr::new("project-review") => {
            run_review_operation(ReviewOperation::Project, request, stdout, stderr)
        },
        [command, request] if command == OsStr::new("build-review") => {
            run_review_operation(ReviewOperation::Build, request, stdout, stderr)
        },
        [command, request] if command == OsStr::new("approve") => {
            run_review_operation(ReviewOperation::Approve, request, stdout, stderr)
        },
        [command, request] if command == OsStr::new("create-checkpoint") => {
            run_checkpoint_operation(CheckpointOperation::Create, request, stdout, stderr)
        },
        [command, request] if command == OsStr::new("propose-activation") => {
            run_checkpoint_operation(CheckpointOperation::Activate, request, stdout, stderr)
        },
        [command, output] if command == OsStr::new("prepare-activation") => {
            run_prepare_activation(output, stdout, stderr)
        },
        [command, request] if command == OsStr::new("resolve-context") => {
            run_context_operation(request, stdout, stderr)
        },
        [command, activation, request]
            if command == OsStr::new("resolve-activation-review-context") =>
        {
            run_prospective_context_operation(activation, request, stdout, stderr)
        },
        [command, request, build_id] if command == OsStr::new("verify-context-build") => {
            run_context_verification(request, build_id, stdout, stderr)
        },
        [command, request] if command == OsStr::new("refresh-context-manifests") => {
            run_refresh_context_manifests(request, stdout, stderr)
        },
        _ => write_text(stderr, UNSUPPORTED_COMMAND, ExitCode::from(2)),
    }
}

#[derive(Serialize)]
struct ArgumentFailure {
    schema: &'static str,
    ok: bool,
    error: ArgumentError,
}

#[derive(Serialize)]
struct ArgumentError {
    code: &'static str,
    affected_ids: Vec<String>,
    next_actions: Vec<&'static str>,
}

fn argument_failure(code: &'static str, affected_ids: Vec<String>) -> ArgumentFailure {
    ArgumentFailure {
        schema: "methexis.error/v1alpha1",
        ok: false,
        error: ArgumentError {
            code,
            affected_ids,
            next_actions: vec!["methexis --help"],
        },
    }
}

fn run_prepare_approval(
    args: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let Ok(parsed) = parse_prepare_approval(args) else {
        return write_json(
            stderr,
            &argument_failure("invalid_prepare_arguments", Vec::new()),
            ExitCode::from(2),
        );
    };
    let root = env::current_dir()?;
    let service = ReviewService::new(&root);
    let result = match &parsed.target {
        PrepareApprovalTarget::Manifest(manifest) => service.prepare_approval(
            Path::new(manifest),
            &parsed.reviewer,
            parsed.replace_current,
        ),
        PrepareApprovalTarget::Canonical {
            knowledge_id,
            revision,
        } => service.prepare_canonical_approval(
            knowledge_id,
            revision,
            &parsed.reviewer,
            parsed.replace_current,
        ),
    };
    match result {
        Ok(request) => write_json_pretty(stdout, &request, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PrepareApprovalTarget {
    Manifest(OsString),
    Canonical {
        knowledge_id: String,
        revision: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
struct PrepareApprovalArgs {
    target: PrepareApprovalTarget,
    reviewer: String,
    replace_current: bool,
}

fn parse_prepare_approval(args: &[OsString]) -> Result<PrepareApprovalArgs, ()> {
    let mut manifest = None;
    let mut canonical = None;
    let mut revision = None;
    let mut reviewer = None;
    let mut replace_current = false;
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].to_str().ok_or(())?;
        if argument == "--replace-current" {
            if replace_current {
                return Err(());
            }
            replace_current = true;
            index += 1;
            continue;
        }
        let (kind, value) =
            if argument == "--reviewer" || argument == "--canonical" || argument == "--revision" {
                index += 1;
                let value = args.get(index).and_then(|value| value.to_str()).ok_or(())?;
                if value.starts_with("--") {
                    return Err(());
                }
                (argument, value)
            } else if let Some(value) = argument.strip_prefix("--reviewer=") {
                ("--reviewer", value)
            } else if let Some(value) = argument.strip_prefix("--canonical=") {
                ("--canonical", value)
            } else if let Some(value) = argument.strip_prefix("--revision=") {
                ("--revision", value)
            } else if argument.starts_with("--") {
                return Err(());
            } else {
                if manifest.replace(args[index].clone()).is_some() {
                    return Err(());
                }
                index += 1;
                continue;
            };
        if value.is_empty() {
            return Err(());
        }
        let slot = match kind {
            "--reviewer" => &mut reviewer,
            "--canonical" => &mut canonical,
            "--revision" => &mut revision,
            _ => unreachable!(),
        };
        if slot.replace(value.to_owned()).is_some() {
            return Err(());
        }
        index += 1;
    }
    let reviewer = reviewer.ok_or(())?;
    let target = match (manifest, canonical, revision) {
        (Some(manifest), None, None) => PrepareApprovalTarget::Manifest(manifest),
        (None, Some(knowledge_id), Some(revision)) => PrepareApprovalTarget::Canonical {
            knowledge_id,
            revision,
        },
        _ => return Err(()),
    };
    Ok(PrepareApprovalArgs {
        target,
        reviewer,
        replace_current,
    })
}

#[cfg(test)]
mod prepare_approval_argument_tests {
    use std::ffi::OsString;

    use super::parse_prepare_approval;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    // 분리형 --reviewer 뒤의 구조 옵션이나 알 수 없는 장기 옵션은 reviewer 값이 아니라
    // 누락된 값과 독립 옵션으로 취급해 인자 단계에서 닫습니다.
    #[test]
    fn separated_reviewer_rejects_flag_like_values() {
        for values in [
            &["manifest.json", "--reviewer", "--replace-current"][..],
            &["manifest.json", "--reviewer", "--reviewer=owner"][..],
            &["manifest.json", "--reviewer", "--unknown"][..],
        ] {
            assert!(parse_prepare_approval(&args(values)).is_err(), "{values:?}");
        }
    }

    // 선행 dash를 실제 OwnerId 문자로 전달해야 하는 명시적 equals form은 파서에서
    // 보존하고, 존재 여부는 기존 prepare 서비스 검증에 맡깁니다.
    #[test]
    fn equals_reviewer_preserves_a_leading_dash_literal() {
        let parsed =
            parse_prepare_approval(&args(&["manifest.json", "--reviewer=--literal-owner"]))
                .unwrap();

        assert_eq!(parsed.reviewer, "--literal-owner");
        assert!(!parsed.replace_current);
    }

    // 동일 옵션의 분리형·equals form 혼용은 마지막 값을 덮어쓰지 않고 기존 중복 오류를
    // 유지합니다.
    #[test]
    fn duplicate_reviewer_forms_remain_invalid() {
        assert!(
            parse_prepare_approval(&args(&[
                "manifest.json",
                "--reviewer",
                "owner",
                "--reviewer=other",
            ]))
            .is_err()
        );
    }

    // canonical form은 ID와 revision을 한 쌍으로 요구하고 positional manifest와 섞이지 않는다.
    #[test]
    fn canonical_target_requires_an_exact_revision_and_no_manifest() {
        let parsed = parse_prepare_approval(&args(&[
            "--canonical",
            "tui.relocated",
            "--revision=sha256:1234",
            "--reviewer",
            "tui-architecture",
        ]))
        .unwrap();
        assert_eq!(
            parsed.target,
            super::PrepareApprovalTarget::Canonical {
                knowledge_id: "tui.relocated".to_owned(),
                revision: "sha256:1234".to_owned(),
            }
        );
        assert!(
            parse_prepare_approval(&args(&[
                "manifest.json",
                "--canonical=tui.relocated",
                "--revision=sha256:1234",
                "--reviewer=owner",
            ]))
            .is_err()
        );
    }
}

fn run_prepare_checkpoint(
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let service = CheckpointService::new(&root);
    match service.prepare_checkpoint() {
        Ok(request) => write_json_pretty(stdout, &request, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

fn run_prepare_activation(
    output: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let service = CheckpointService::new(&root);
    match service.prepare_activation(Path::new(output)) {
        Ok(request) => write_json_pretty(stdout, &request, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

fn run_check(
    args: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    if args == [OsString::from("--staged-activation")] {
        let root = env::current_dir()?;
        let service = CheckpointService::new(&root);
        return match service.check_staged_transition() {
            Ok(StagedTransition::Prospective(report)) => {
                write_json(stdout, &report, ExitCode::SUCCESS)
            },
            Ok(StagedTransition::Ordinary(fallback)) => {
                let report = check_repository_selected(&root, &CheckClass::ALL);
                if let Err(error) = service.finish_staged_fallback(fallback) {
                    write_json(stderr, &error, ExitCode::from(2))
                } else if report.ok {
                    write_json(stdout, &report, ExitCode::SUCCESS)
                } else {
                    write_json(stderr, &report, ExitCode::from(2))
                }
            },
            Err(error) => write_json(stderr, &error, ExitCode::from(2)),
        };
    }
    let selection = match parse_check_selection(args) {
        Ok(selection) => selection,
        Err(()) => {
            return write_json(
                stderr,
                &argument_failure("invalid_check_selector", Vec::new()),
                ExitCode::from(2),
            );
        },
    };
    if selection.unit.is_some()
        && (!selection.summary
            || !selection
                .requested
                .iter()
                .any(|check| matches!(check, CheckClass::Authority | CheckClass::Artifacts)))
    {
        return write_json(
            stderr,
            &ArgumentFailure {
                schema: "methexis.error/v1alpha1",
                ok: false,
                error: ArgumentError {
                    code: "invalid_check_selector",
                    affected_ids: selection.unit.into_iter().collect(),
                    next_actions: vec![
                        "use --unit with --summary and --only authority or artifacts",
                    ],
                },
            },
            ExitCode::from(2),
        );
    }
    let root = env::current_dir()?;
    let mut report = check_repository_selected(&root, &selection.requested);
    if !report.ok {
        return write_json(stderr, &report, ExitCode::from(2));
    }
    if let Some(unit) = selection.unit.as_deref() {
        report.units.retain(|candidate| candidate.id == unit);
        if report.units.is_empty() {
            return write_json(
                stderr,
                &ArgumentFailure {
                    schema: "methexis.error/v1alpha1",
                    ok: false,
                    error: ArgumentError {
                        code: "unknown_check_unit",
                        affected_ids: vec![unit.to_owned()],
                        next_actions: vec!["choose an id reported by methexis check"],
                    },
                },
                ExitCode::from(2),
            );
        }
    } else if selection.summary {
        report.units.clear();
    }
    if selection.summary {
        write_json(stdout, &CheckSummary::from(&report), ExitCode::SUCCESS)
    } else {
        write_json(stdout, &report, ExitCode::SUCCESS)
    }
}

struct CheckSelection {
    requested: Vec<CheckClass>,
    summary: bool,
    unit: Option<String>,
}

fn parse_check_selection(args: &[OsString]) -> Result<CheckSelection, ()> {
    let mut summary = false;
    let mut unit = None;

    let mut requested = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].to_str().ok_or(())?;
        if argument == "--summary" {
            summary = true;
            index += 1;
            continue;
        }
        if argument == "--unit" {
            index += 1;
            let value = args.get(index).and_then(|value| value.to_str()).ok_or(())?;
            if value.is_empty() || unit.replace(value.to_owned()).is_some() {
                return Err(());
            }
            index += 1;
            continue;
        }
        if let Some(value) = argument.strip_prefix("--unit=") {
            if value.is_empty() || unit.replace(value.to_owned()).is_some() {
                return Err(());
            }
            index += 1;
            continue;
        }
        let value = if argument == "--only" {
            index += 1;
            args.get(index).and_then(|value| value.to_str()).ok_or(())?
        } else if let Some(value) = argument.strip_prefix("--only=") {
            value
        } else {
            return Err(());
        };
        for selector in value.split(',') {
            let selector = selector.trim();
            if selector.is_empty() {
                return Err(());
            }
            requested.push(CheckClass::parse(selector).ok_or(())?);
        }
        index += 1;
    }
    if requested.is_empty() {
        requested.extend(CheckClass::ALL);
    }
    Ok(CheckSelection {
        requested,
        summary,
        unit,
    })
}

#[derive(Serialize)]
struct CheckSummary<'report> {
    schema: &'static str,
    ok: bool,
    requested_checks: &'report [CheckClass],
    executed_checks: &'report [CheckClass],
    checks: &'report [crate::CheckOutcome],
    authority: &'static str,
    affected_ids: &'report [String],
    units: &'report [crate::UnitRevision],
    diagnostic_count: usize,
}

impl<'report> From<&'report crate::CheckReport> for CheckSummary<'report> {
    fn from(report: &'report crate::CheckReport) -> Self {
        Self {
            schema: "methexis.check-summary/v1alpha1",
            ok: report.ok,
            requested_checks: &report.requested_checks,
            executed_checks: &report.executed_checks,
            checks: &report.checks,
            authority: report.authority,
            affected_ids: &report.affected_ids,
            units: &report.units,
            diagnostic_count: report.diagnostics.len(),
        }
    }
}

fn run_context_operation(
    request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let result = ContextService::new(&root).resolve(Path::new(request));
    match result {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

fn run_prospective_context_operation(
    activation_request: &OsStr,
    context_request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let result = ContextService::new(&root)
        .resolve_activation_review(Path::new(activation_request), Path::new(context_request));
    match result {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

fn run_context_verification(
    request: &OsStr,
    build_id: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let Some(build_id) = build_id.to_str() else {
        return write_json(
            stderr,
            &argument_failure("invalid_verify_arguments", Vec::new()),
            ExitCode::from(2),
        );
    };
    let root = env::current_dir()?;
    let result = ContextService::new(&root).verify(Path::new(request), build_id);
    match result {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

fn run_refresh_context_manifests(
    request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let service = ContextService::new(&root);
    match service.refresh_manifests(Path::new(request)) {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

enum CheckpointOperation {
    Create,
    Activate,
}

fn run_checkpoint_operation(
    operation: CheckpointOperation,
    request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let service = CheckpointService::new(&root);
    let request = Path::new(request);
    let result = match operation {
        CheckpointOperation::Create => service.create(request),
        CheckpointOperation::Activate => service.propose_activation(request),
    };
    match result {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

enum ReviewOperation {
    Project,
    Build,
    Approve,
}

fn run_author_operation(
    request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let service = AuthorService::new(&root);
    match service.author_revision(Path::new(request)) {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

fn run_review_operation(
    operation: ReviewOperation,
    request: &OsStr,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<ExitCode> {
    let root = env::current_dir()?;
    let service = ReviewService::new(&root);
    let request = Path::new(request);
    let result = match operation {
        ReviewOperation::Project => service.generate_projection(request),
        ReviewOperation::Build => service.build_review(request),
        ReviewOperation::Approve => service.record_approval(request),
    };
    match result {
        Ok(result) => write_json(stdout, &result, ExitCode::SUCCESS),
        Err(error) => write_json(stderr, &error, ExitCode::from(2)),
    }
}

fn write_text(writer: &mut impl Write, text: &str, exit_code: ExitCode) -> io::Result<ExitCode> {
    writer.write_all(text.as_bytes())?;
    Ok(exit_code)
}

fn write_json(
    writer: &mut impl Write,
    value: &impl Serialize,
    exit_code: ExitCode,
) -> io::Result<ExitCode> {
    write_json_with(writer, value, exit_code, false)
}

fn write_json_pretty(
    writer: &mut impl Write,
    value: &impl Serialize,
    exit_code: ExitCode,
) -> io::Result<ExitCode> {
    write_json_with(writer, value, exit_code, true)
}

fn write_json_with(
    writer: &mut impl Write,
    value: &impl Serialize,
    exit_code: ExitCode,
    pretty: bool,
) -> io::Result<ExitCode> {
    if pretty {
        serde_json::to_writer_pretty(&mut *writer, value).map_err(io::Error::other)?;
    } else {
        serde_json::to_writer(&mut *writer, value).map_err(io::Error::other)?;
    }
    writer.write_all(b"\n")?;
    Ok(exit_code)
}
