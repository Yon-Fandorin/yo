use std::{path::Path, process::Stdio};

use crate::{git, impact::deferred_branch};

const MESSAGE_ENVIRONMENT: &str = "YO_XTASK_ACCEPTED_COMMIT_MESSAGE";
const EDITOR_COMMAND: &str = "__accepted-commit-message-editor";

pub(super) fn create(repository: &Path, message: &Path) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("cannot locate the xtask executable: {error}"))?;
    create_with_editor(repository, message, &executable)
}

pub(super) fn create_with_editor(
    repository: &Path,
    message: &Path,
    executable: &Path,
) -> Result<(), String> {
    let branch = git::optional_output_in(
        repository,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        false,
    )?
    .unwrap_or_default();
    if branch.trim().is_empty() || deferred_branch(branch.trim()) {
        return Err(
            "cargo xtask slice commit is reserved for an attached accepted integration branch"
                .to_owned(),
        );
    }
    if git::optional_output_in(repository, &["config", "--get", "commit.template"], false)?
        .is_some_and(|value| !value.trim().is_empty())
    {
        return Err(
            "cargo xtask slice commit requires commit.template to be unset so Git reports an \
             unambiguous new-commit source"
                .to_owned(),
        );
    }

    let message = message.canonicalize().map_err(|error| {
        format!(
            "cannot resolve prepared commit message {}: {error}",
            message.display()
        )
    })?;
    let editor = format!("{} {EDITOR_COMMAND}", shell_quote(executable)?);
    let status = git::command_in(repository, false)
        .args(["commit", "--no-amend", "--no-template"])
        .env("GIT_EDITOR", editor)
        .env(MESSAGE_ENVIRONMENT, message)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("cannot run accepted git commit: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("accepted git commit failed with {status}"))
    }
}

pub(super) fn copy_message(target: &Path) -> Result<(), String> {
    let source = std::env::var_os(MESSAGE_ENVIRONMENT)
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "accepted-commit editor is missing its prepared message".to_owned())?;
    std::fs::copy(&source, target).map_err(|error| {
        format!(
            "cannot copy prepared commit message {} to {}: {error}",
            source.display(),
            target.display()
        )
    })?;
    Ok(())
}

fn shell_quote(path: &Path) -> Result<String, String> {
    let value = path
        .to_str()
        .ok_or_else(|| "xtask executable path must be valid UTF-8".to_owned())?;
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}
