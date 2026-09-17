use std::{collections::BTreeSet, path::Path, str};

use super::model::{
    REQUEST_SCHEMA_V1_ALPHA4, REQUEST_SCHEMA_V1_ALPHA5, REQUEST_SCHEMA_V1_ALPHA6,
    REQUEST_SCHEMA_V1_ALPHA7, Request,
};
use crate::{git, slice_contract};

pub(super) fn apply_repository_authority_policy(
    repository: &Path,
    bound: &slice_contract::BoundSlice,
    request: &mut Request,
) -> Result<(), String> {
    let policy = match request.schema.as_str() {
        REQUEST_SCHEMA_V1_ALPHA4 => AuthorityPolicy::V1Alpha1,
        REQUEST_SCHEMA_V1_ALPHA5 | REQUEST_SCHEMA_V1_ALPHA6 | REQUEST_SCHEMA_V1_ALPHA7 => {
            AuthorityPolicy::V1Alpha2
        },
        _ => return Ok(()),
    };
    let range = format!("{}..HEAD", bound.base);
    let changed = git::output_bytes_in(repository, &["diff", "--name-only", "-z", &range], false)?;
    let changed = changed
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            str::from_utf8(path).map(str::to_owned).map_err(|_| {
                "changed path is not UTF-8; repository authority routing is ambiguous".to_owned()
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    request.repository_authority_paths = match policy {
        AuthorityPolicy::V1Alpha1 => authority_paths_for_changed_paths_v1alpha1(&changed),
        AuthorityPolicy::V1Alpha2 => authority_paths_for_changed_paths_v1alpha2(&changed),
    };
    Ok(())
}

#[derive(Clone, Copy)]
enum AuthorityPolicy {
    V1Alpha1,
    V1Alpha2,
}

pub(super) fn authority_paths_for_changed_paths_v1alpha1(changed: &[String]) -> Vec<String> {
    let workflow_changed = changed.iter().any(|path| {
        path == "CONTRIBUTING.md"
            || path == "AGENTS.md"
            || path.starts_with("CONTRIBUTING/")
            || path.starts_with("tools/xtask/")
            || path.starts_with("tools/validation/")
            || path.starts_with(".github/")
    });
    let mut authorities = BTreeSet::from(["AGENTS.md".to_owned()]);
    if workflow_changed {
        authorities.insert("CONTRIBUTING.md".to_owned());
    }
    for path in changed {
        if path == "AGENTS.md" || path.ends_with("/AGENTS.md") {
            authorities.insert(path.clone());
        }
    }
    authorities.into_iter().collect()
}

const CONTRIBUTOR_AUTHORITY: &str = "CONTRIBUTING.md";
const FORMAL_AUTHORITY: &str = "CONTRIBUTING/formal-slices.md";
const PACKET_AUTHORITY: &str = "CONTRIBUTING/review-packets.md";
const DELIVERY_AUTHORITY: &str = "CONTRIBUTING/review-delivery.md";
const INTEGRATION_AUTHORITY: &str = "CONTRIBUTING/review-and-integration.md";

pub(super) fn authority_paths_for_changed_paths_v1alpha2(changed: &[String]) -> Vec<String> {
    let mut authorities = BTreeSet::from(["AGENTS.md".to_owned()]);
    let mut has_specific_owner = false;
    let mut neutral_facade = false;
    let mut ambiguous_shared_workflow = false;

    for path in changed {
        if path == "AGENTS.md" || path.ends_with("/AGENTS.md") {
            authorities.insert(path.clone());
            continue;
        }
        if let Some(owners) = shared_workflow_owners(path) {
            authorities.extend(owners.iter().map(|owner| (*owner).to_owned()));
            has_specific_owner = true;
        } else if let Some(owner) =
            exact_authority_owner(path).or_else(|| workflow_code_owner(path))
        {
            authorities.insert(owner.to_owned());
            has_specific_owner = true;
        } else if is_neutral_workflow_facade(path) {
            neutral_facade = true;
        } else if is_ambiguous_shared_workflow(path) {
            ambiguous_shared_workflow = true;
        }
    }

    if ambiguous_shared_workflow || neutral_facade && !has_specific_owner {
        authorities.extend([
            CONTRIBUTOR_AUTHORITY.to_owned(),
            FORMAL_AUTHORITY.to_owned(),
            PACKET_AUTHORITY.to_owned(),
            DELIVERY_AUTHORITY.to_owned(),
            INTEGRATION_AUTHORITY.to_owned(),
        ]);
    }
    authorities.into_iter().collect()
}

fn exact_authority_owner(path: &str) -> Option<&'static str> {
    match path {
        "CONTRIBUTING.md" => Some(CONTRIBUTOR_AUTHORITY),
        FORMAL_AUTHORITY => Some(FORMAL_AUTHORITY),
        PACKET_AUTHORITY => Some(PACKET_AUTHORITY),
        DELIVERY_AUTHORITY => Some(DELIVERY_AUTHORITY),
        INTEGRATION_AUTHORITY => Some(INTEGRATION_AUTHORITY),
        _ if path.starts_with("CONTRIBUTING/") => Some(CONTRIBUTOR_AUTHORITY),
        _ => None,
    }
}

fn shared_workflow_owners(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "tools/xtask/src/review_protocol.rs" => {
            Some(&[PACKET_AUTHORITY, DELIVERY_AUTHORITY, INTEGRATION_AUTHORITY])
        },
        "tools/xtask/src/review_result.rs" => Some(&[PACKET_AUTHORITY, INTEGRATION_AUTHORITY]),
        _ if path.starts_with("tools/xtask/src/review_result/") => {
            Some(&[PACKET_AUTHORITY, INTEGRATION_AUTHORITY])
        },
        _ => None,
    }
}

fn workflow_code_owner(path: &str) -> Option<&'static str> {
    if matches!(
        path,
        "tools/xtask/src/impact/change.rs" | "tools/context.py" | "tools/test_context.py"
    ) || matches!(
        path,
        "tools/chat_preview.py"
            | "tools/test_chat_preview.py"
            | "tools/clipboard_bridge.py"
            | "tools/test_clipboard_bridge.py"
            | "tools/test_ssh_clipboard_capture.py"
            | "crates/yo-cli/src/execution/image/clipboard/ssh_capture.py"
    ) {
        Some(CONTRIBUTOR_AUTHORITY)
    } else if path.starts_with("tools/xtask/src/review_packet/")
        || path == "tools/xtask/src/review_prepare.rs"
        || path.starts_with("tools/xtask/src/review_prepare/")
        || path.starts_with("tools/xtask/src/review_delta/")
        || path == "tools/xtask/src/validation_summary.rs"
        || path.starts_with("tools/xtask/src/validation_summary/")
        || matches!(
            path,
            "tools/validation/bounded-run.sh" | "tools/validation/bounded-run-tests.sh"
        )
        || matches!(
            path,
            "tools/validation/codex-interview-resume.py"
                | "tools/validation/codex-policy-persistence.py"
                | "tools/validation/managed-start-failure.py"
        )
    {
        Some(PACKET_AUTHORITY)
    } else if path == "tools/xtask/src/review_delivery.rs"
        || path.starts_with("tools/xtask/src/review_delivery/")
        || path.starts_with("tools/xtask/src/review_egress/")
        || path.starts_with("tools/xtask/src/review_target_admission/")
        || path.starts_with("tools/xtask/src/review_continuation_preflight/")
        || matches!(
            path,
            "tools/xtask/src/review_continuation_preflight.rs"
                | "tools/xtask/src/review_session.rs"
        )
    {
        Some(DELIVERY_AUTHORITY)
    } else if path.starts_with("tools/xtask/src/slice_gate/")
        || path.starts_with("tools/xtask/src/slice_accept/")
        || path.starts_with("tools/xtask/src/slice_close/")
        || path.starts_with("tools/xtask/src/slice_status/")
        || path == "tools/xtask/src/slice_status.rs"
        || path.starts_with("tools/xtask/src/impact/")
        || matches!(
            path,
            "tools/xtask/src/slice_accept.rs" | "tools/xtask/src/cost_report.rs"
        )
    {
        Some(INTEGRATION_AUTHORITY)
    } else if path.starts_with("tools/xtask/src/slice_contract/")
        || path.starts_with("tools/xtask/src/slice_create/")
        || path.starts_with("tools/xtask/src/activation_slice/")
        || path == "tools/xtask/src/slice_worktree.rs"
    {
        Some(FORMAL_AUTHORITY)
    } else if matches!(
        path,
        "tools/xtask/src/validation_stage.rs" | "tools/xtask/src/test_explanations.rs"
    ) || matches!(
        path,
        "tools/validation/developer-docs-build.sh"
            | "tools/validation/developer-docs.sh"
            | "tools/validation/developer-docs-translations-tests.sh"
            | "tools/validation/developer-docs-translations.sh"
            | "tools/validation/yo-cli-unix-matrix-tests.sh"
            | "tools/validation/yo-cli-unix-matrix.sh"
    ) || path.starts_with(".github/")
    {
        Some(CONTRIBUTOR_AUTHORITY)
    } else {
        None
    }
}

fn is_neutral_workflow_facade(path: &str) -> bool {
    matches!(
        path,
        "tools/xtask/src/lib.rs"
            | "tools/xtask/src/main.rs"
            | "tools/xtask/Cargo.toml"
            | "tools/xtask/src/cli.rs"
    ) || path.starts_with("tools/xtask/src/cli/")
}

fn is_ambiguous_shared_workflow(path: &str) -> bool {
    path.starts_with("tools/xtask/")
        || path.starts_with("tools/validation/")
        || matches!(
            path,
            "tools/xtask/src/bounded_file.rs"
                | "tools/xtask/src/git.rs"
                | "tools/xtask/src/test_support.rs"
                | "Cargo.toml"
                | "Cargo.lock"
                | "hk.pkl"
        )
}
