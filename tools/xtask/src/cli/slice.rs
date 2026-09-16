mod lifecycle;
mod review;

use std::ffi::{OsStr, OsString};

pub(super) fn run(
    scope: &OsStr,
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    match scope.to_str() {
        Some("create")
        | Some("create-activation")
        | Some("close")
        | Some("gate")
        | Some("commit")
        | Some("accept")
        | Some("status") => lifecycle::run(scope, arguments),
        Some("review-packet")
        | Some("review-prepare")
        | Some("review-delta")
        | Some("review-deliver")
        | Some("review-continuation-preflight")
        | Some("review-result-correction-preflight")
        | Some("cost-report")
        | Some("review-egress")
        | Some("review-target-admission") => review::run(scope, arguments),
        _ => Err(super::general_usage()),
    }
}

pub(super) fn run_accepted_commit_message_editor(
    target: &OsStr,
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    lifecycle::run_accepted_commit_message_editor(target, arguments)
}
