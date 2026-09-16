mod check;
mod docs;
mod slice;
mod slice_contract;

#[cfg(test)]
mod tests;

use std::ffi::OsString;

pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut arguments = arguments.into_iter();
    let command = arguments.next();
    let scope = arguments.next();
    match (command.as_deref(), scope.as_deref()) {
        (Some(command), Some(target)) if command == "__accepted-commit-message-editor" => {
            slice::run_accepted_commit_message_editor(target, &mut arguments)
        },
        (Some(command), Some(scope)) if command == "slice" => slice::run(scope, &mut arguments),
        (Some(command), Some(action)) if command == "docs" => docs::run(action, &mut arguments),
        (Some(command), Some(action)) if command == "slice-contract" => {
            slice_contract::run(action, &mut arguments)
        },
        (Some(command), Some(check)) if command == "check" => check::run(check, &mut arguments),
        _ => Err(general_usage()),
    }
}

fn current_repository() -> Result<std::path::PathBuf, String> {
    std::env::current_dir().map_err(|error| format!("cannot locate the repository: {error}"))
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
     cargo xtask slice review-result-correction-preflight <request.json>\n\
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
     cargo xtask check <change-preflight|commit-preflight|developer-docs-impact|slice-review-impact> \
     <commit-message-file> [changed-paths-file] [branch]"
        .to_owned()
}
