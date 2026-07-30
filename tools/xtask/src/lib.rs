mod git;
mod impact;

use std::{ffi::OsString, path::PathBuf};

use impact::ImpactInput;

pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut arguments = arguments.into_iter();
    match (arguments.next().as_deref(), arguments.next().as_deref()) {
        (Some(command), Some(check)) if command == "check" => {
            let check = check.to_string_lossy();
            let head_fallback = match check.as_ref() {
                "developer-docs-impact" => false,
                "slice-review-impact" => true,
                _ => return Err(usage(check.as_ref())),
            };
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
    format!(
        "usage: cargo xtask check {} <commit-message-file> [changed-paths-file] [branch]",
        check
    )
}

fn general_usage() -> String {
    "usage: cargo xtask check <developer-docs-impact|slice-review-impact> \
     <commit-message-file> [changed-paths-file] [branch]"
        .to_owned()
}
