use crate::review_protocol;

pub(super) fn verify_common(
    schema: &str,
    expected_schema: &str,
    name: &str,
    expected_name: &str,
    status: &str,
    exit_code: i32,
) -> Result<(), String> {
    if schema != expected_schema {
        return Err(format!("schema must be `{expected_schema}`"));
    }
    if name != expected_name {
        return Err(format!(
            "summary name `{name}` does not match requested evidence name `{expected_name}`"
        ));
    }
    if !matches!(status, "passed" | "failed") || (status == "passed") != (exit_code == 0) {
        return Err("status and exit_code are inconsistent".to_owned());
    }
    Ok(())
}

pub(super) fn verify_exact_execution(
    head_commit: &str,
    worktree_state: &str,
    reused: bool,
    candidate: &str,
) -> Result<(), String> {
    review_protocol::require_commit(head_commit, "validation summary head_commit")?;
    if head_commit != candidate {
        return Err(format!(
            "head_commit {head_commit} does not match candidate {candidate}"
        ));
    }
    if worktree_state != "clean" {
        return Err("worktree_state must be `clean`".to_owned());
    }
    if reused {
        return Err("an execution summary must record `reused:false`".to_owned());
    }
    Ok(())
}
