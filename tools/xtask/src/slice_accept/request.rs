use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{bounded_file, review_protocol, slice_gate};

pub(super) const REQUEST_LIMIT: usize = 64 * 1024;
pub(super) const ACCEPT_REQUEST_SCHEMA: &str = "yo.slice-accept-request/v1alpha1";
pub(super) const ACCEPT_REQUEST_SCHEMA_V1_ALPHA2: &str = "yo.slice-accept-request/v1alpha2";
pub(super) const ACCEPT_REQUEST_SCHEMA_V1_ALPHA3: &str = "yo.slice-accept-request/v1alpha3";
pub(super) const ACCEPT_RESULT_SCHEMA: &str = "yo.slice-accept-result/v1alpha1";
pub(super) const ACCEPT_RESULT_SCHEMA_V1_ALPHA2: &str = "yo.slice-accept-result/v1alpha2";
pub(super) const ACCEPT_RESULT_SCHEMA_V1_ALPHA3: &str = "yo.slice-accept-result/v1alpha3";
pub(super) const COMMIT_GIT_HOOKS: &str = "git_hooks";
pub(super) const COMMIT_CANDIDATE_HK_RECEIPT: &str = "candidate_hk_receipt";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AcceptRequest {
    pub(super) schema: String,
    pub(super) slice: String,
    pub(super) gate_request_path: String,
    pub(super) gate_request_hash: String,
    pub(super) message_source_path: String,
    pub(super) message_source_hash: String,
    pub(super) message_output_path: String,
    pub(super) close_prepare_request_path: String,
    pub(super) close_prepare_request_hash: String,
    pub(super) close_plan_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) push: Option<Push>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) commit_verification: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) approval_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) effect_scope: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Push {
    pub(super) remote: String,
    pub(super) reference: String,
}

pub(super) fn validate_accept_request(request: &AcceptRequest) -> Result<(), String> {
    match request.schema.as_str() {
        ACCEPT_REQUEST_SCHEMA
            if request.approval_scope.is_some()
                && request.effect_scope.is_none()
                && request.push.is_some()
                && request.commit_verification.is_none() => {},
        ACCEPT_REQUEST_SCHEMA_V1_ALPHA2
            if request.approval_scope.is_none()
                && request.effect_scope.is_some()
                && request.push.is_some()
                && request.commit_verification.is_none() => {},
        ACCEPT_REQUEST_SCHEMA_V1_ALPHA3
            if request.approval_scope.is_none()
                && request.effect_scope.is_some()
                && request.commit_verification.as_deref().is_some_and(|mode| {
                    matches!(mode, COMMIT_GIT_HOOKS | COMMIT_CANDIDATE_HK_RECEIPT)
                }) => {},
        ACCEPT_REQUEST_SCHEMA => {
            return Err(
                "yo.slice-accept-request/v1alpha1 requires approval_scope and forbids effect_scope"
                    .to_owned(),
            );
        },
        ACCEPT_REQUEST_SCHEMA_V1_ALPHA2 => {
            return Err(
                "yo.slice-accept-request/v1alpha2 requires effect_scope and forbids approval_scope"
                    .to_owned(),
            );
        },
        ACCEPT_REQUEST_SCHEMA_V1_ALPHA3 => {
            return Err(
                "yo.slice-accept-request/v1alpha3 requires effect_scope and one closed commit_verification mode, and forbids approval_scope"
                    .to_owned(),
            );
        },
        _ => {
            return Err(format!(
                "unsupported Slice accept request schema `{}`; expected `{ACCEPT_REQUEST_SCHEMA}`, `{ACCEPT_REQUEST_SCHEMA_V1_ALPHA2}`, or `{ACCEPT_REQUEST_SCHEMA_V1_ALPHA3}`",
                request.schema
            ));
        },
    }
    for (value, label) in [
        (&request.gate_request_path, "gate_request_path"),
        (&request.message_source_path, "message_source_path"),
        (&request.message_output_path, "message_output_path"),
        (
            &request.close_prepare_request_path,
            "close_prepare_request_path",
        ),
        (&request.close_plan_path, "close_plan_path"),
    ] {
        if value.is_empty() || value.len() > 4096 || value.contains('\0') {
            return Err(format!("{label} must be a non-empty bounded path"));
        }
    }
    for (value, label) in [
        (&request.gate_request_hash, "gate_request_hash"),
        (&request.message_source_hash, "message_source_hash"),
        (
            &request.close_prepare_request_hash,
            "close_prepare_request_hash",
        ),
    ] {
        if value.len() != 71
            || !value.starts_with("sha256:")
            || !value[7..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(format!("{label} must be sha256:<64 lowercase hex>"));
        }
    }
    if let Some(push) = &request.push
        && (push.remote.is_empty()
            || push.remote.len() > 64
            || !push
                .remote
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    {
        return Err("push remote must be one bounded Git remote token".to_owned());
    }
    Ok(())
}

impl AcceptRequest {
    pub(super) fn effect_scope(&self) -> Result<&str, String> {
        match self.schema.as_str() {
            ACCEPT_REQUEST_SCHEMA => self.approval_scope.as_deref(),
            ACCEPT_REQUEST_SCHEMA_V1_ALPHA2 | ACCEPT_REQUEST_SCHEMA_V1_ALPHA3 => {
                self.effect_scope.as_deref()
            },
            _ => None,
        }
        .ok_or_else(|| "Slice accept request has no version-matched effect scope".to_owned())
    }

    pub(super) fn result_schema(&self) -> &'static str {
        match self.schema.as_str() {
            ACCEPT_REQUEST_SCHEMA_V1_ALPHA2 => ACCEPT_RESULT_SCHEMA_V1_ALPHA2,
            ACCEPT_REQUEST_SCHEMA_V1_ALPHA3 => ACCEPT_RESULT_SCHEMA_V1_ALPHA3,
            _ => ACCEPT_RESULT_SCHEMA,
        }
    }

    pub(super) fn commit_verification(&self) -> Result<&str, String> {
        match self.schema.as_str() {
            ACCEPT_REQUEST_SCHEMA | ACCEPT_REQUEST_SCHEMA_V1_ALPHA2 => Ok(COMMIT_GIT_HOOKS),
            ACCEPT_REQUEST_SCHEMA_V1_ALPHA3 => self
                .commit_verification
                .as_deref()
                .ok_or_else(|| "fast Slice acceptance has no commit verification mode".to_owned()),
            _ => Err("unsupported Slice accept request schema".to_owned()),
        }
    }

    pub(super) fn expected_effect_scope(
        &self,
        candidate: &str,
        reference: &str,
    ) -> Result<String, String> {
        match self.schema.as_str() {
            ACCEPT_REQUEST_SCHEMA | ACCEPT_REQUEST_SCHEMA_V1_ALPHA2 => {
                let push = self
                    .push
                    .as_ref()
                    .ok_or_else(|| "legacy Slice acceptance requires push".to_owned())?;
                Ok(effect_scope(
                    &self.slice,
                    candidate,
                    &push.remote,
                    reference,
                ))
            },
            ACCEPT_REQUEST_SCHEMA_V1_ALPHA3 => Ok(fast_effect_scope(
                &self.slice,
                candidate,
                self.push.as_ref(),
                reference,
                self.commit_verification()?,
            )),
            _ => Err("unsupported Slice accept request schema".to_owned()),
        }
    }
}

pub(super) fn effect_scope(slice: &str, candidate: &str, remote: &str, reference: &str) -> String {
    format!(
        "yo.slice-accept-effects/v1alpha1;slice={slice};candidate={candidate};squash=true;push={remote}:{reference};close=true"
    )
}

pub(super) fn fast_effect_scope(
    slice: &str,
    candidate: &str,
    push: Option<&Push>,
    reference: &str,
    commit_verification: &str,
) -> String {
    let push = push.map_or_else(
        || "none".to_owned(),
        |push| format!("{}:{reference}", push.remote),
    );
    format!(
        "yo.slice-accept-effects/v1alpha2;slice={slice};candidate={candidate};squash=true;push={push};commit_verification={commit_verification};close=true"
    )
}

pub(super) fn fast_commit_verification(
    gate: &slice_gate::ReadyGate,
    candidate_base: &str,
    integration_head: &str,
) -> &'static str {
    let exact_hk_argv = [
        "hk",
        "check",
        "--check",
        "--from-ref",
        candidate_base,
        "--to-ref",
        gate.candidate_commit.as_str(),
    ];
    let exact_hk_receipt = gate.validation.iter().any(|entry| {
        entry.status == "passed"
            && !entry.reused
            && entry.current_reusable_context
            && entry.argv.iter().map(String::as_str).eq(exact_hk_argv)
    });
    if candidate_base == integration_head && exact_hk_receipt {
        COMMIT_CANDIDATE_HK_RECEIPT
    } else {
        COMMIT_GIT_HOOKS
    }
}

pub(super) fn require_commit_verification(
    requested: &str,
    gate: &slice_gate::ReadyGate,
    candidate_base: &str,
    integration_head: &str,
) -> Result<(), String> {
    let current = fast_commit_verification(gate, candidate_base, integration_head);
    if requested == COMMIT_CANDIDATE_HK_RECEIPT && current != requested {
        Err(
            "candidate_hk_receipt commit verification is no longer eligible; rerun accept preparation so Git hooks remain enabled"
                .to_owned(),
        )
    } else if matches!(requested, COMMIT_GIT_HOOKS | COMMIT_CANDIDATE_HK_RECEIPT) {
        Ok(())
    } else {
        Err("unsupported accepted commit verification mode".to_owned())
    }
}

pub(super) fn require_gate_authorization(
    path: &Path,
    expected: &str,
    accept_schema: &str,
) -> Result<(), String> {
    let bytes = bounded_file::read_regular(path, REQUEST_LIMIT, "Slice gate request")?;
    let gate: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid Slice gate request: {error}"))?;
    let kind = gate
        .pointer("/approval/kind")
        .and_then(serde_json::Value::as_str);
    let scope = gate
        .pointer("/approval/scope")
        .and_then(serde_json::Value::as_str);
    match (accept_schema, kind) {
        (ACCEPT_REQUEST_SCHEMA, Some("exact_candidate"))
        | (ACCEPT_REQUEST_SCHEMA_V1_ALPHA2, Some("exact_candidate"))
        | (ACCEPT_REQUEST_SCHEMA_V1_ALPHA3, Some("exact_candidate"))
            if scope == Some(expected) => Ok(()),
        (ACCEPT_REQUEST_SCHEMA_V1_ALPHA2 | ACCEPT_REQUEST_SCHEMA_V1_ALPHA3, Some("standing_routine"))
            if gate.pointer("/risk/classification").and_then(serde_json::Value::as_str)
                == Some("routine")
                && gate
                    .pointer("/approval/authority")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|authority| authority.starts_with("human/") && authority.len() > 6)
                && scope.is_some_and(|scope| !scope.trim().is_empty())
                && gate.pointer("/approval/candidate_commit").is_none_or(serde_json::Value::is_null)
                && gate.pointer("/approval/diff_hash").is_none_or(serde_json::Value::is_null) =>
        {
            Ok(())
        },
        (ACCEPT_REQUEST_SCHEMA, _) => Err(
            "one-command acceptance v1alpha1 requires the ready gate's exact_candidate approval to name the canonical squash, push, and close effects"
                .to_owned(),
        ),
        (ACCEPT_REQUEST_SCHEMA_V1_ALPHA2, _) => Err(
            "one-command acceptance v1alpha2 requires either the exact canonical effect approval or a satisfied human-origin standing_routine gate"
                .to_owned(),
        ),
        (ACCEPT_REQUEST_SCHEMA_V1_ALPHA3, _) => Err(
            "fast one-command acceptance requires either the exact canonical effect approval or a satisfied human-origin standing_routine gate"
                .to_owned(),
        ),
        _ => Err("unsupported Slice accept request schema for gate authorization".to_owned()),
    }
}

pub(super) fn revalidate_inputs(
    request: &AcceptRequest,
    request_path: &Path,
    request_bytes: &[u8],
    gate: &Path,
    message: &Path,
    close: &Path,
) -> Result<(), String> {
    if bounded_file::read_regular(request_path, REQUEST_LIMIT, "Slice accept request")?
        != request_bytes
    {
        return Err("Slice accept request changed before integration".to_owned());
    }
    require_hash(gate, &request.gate_request_hash, "Slice gate request")?;
    require_hash(
        message,
        &request.message_source_hash,
        "accepted commit message source",
    )?;
    require_hash(
        close,
        &request.close_prepare_request_hash,
        "Slice close preparation request",
    )
}

pub(super) fn require_hash(path: &Path, expected: &str, label: &str) -> Result<(), String> {
    let bytes = bounded_file::read_regular(path, REQUEST_LIMIT, label)?;
    if review_protocol::digest(&bytes) == expected {
        Ok(())
    } else {
        Err(format!(
            "{label} hash differs from the Slice accept request"
        ))
    }
}

pub(super) fn resolve(workspace: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        workspace.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        COMMIT_CANDIDATE_HK_RECEIPT, COMMIT_GIT_HOOKS, effect_scope, fast_commit_verification,
        fast_effect_scope,
    };
    use crate::slice_gate;

    // 한 번의 사용자 결정은 후보뿐 아니라 squash, exact push ref, close까지 모두 같은
    // canonical scope에 포함해야 하며 remote/ref 변화가 곧 다른 승인이 됩니다.
    #[test]
    fn effect_scope_binds_every_orchestrated_mutation() {
        let scope = effect_scope("example", &"a".repeat(40), "origin", "refs/heads/develop");
        assert_eq!(
            scope,
            format!(
                "yo.slice-accept-effects/v1alpha1;slice=example;candidate={};squash=true;push=origin:refs/heads/develop;close=true",
                "a".repeat(40)
            )
        );
        assert_ne!(
            scope,
            effect_scope("example", &"a".repeat(40), "backup", "refs/heads/develop")
        );
    }

    // fast accept의 승인 범위는 push 생략과 commit 검증 방식을 명시적으로 고정하여
    // 로컬 통합 승인이 나중에 원격 효과나 hook 우회로 확대되지 않게 합니다.
    #[test]
    fn fast_effect_scope_binds_no_push_and_commit_verification() {
        let scope = fast_effect_scope(
            "example",
            &"a".repeat(40),
            None,
            "refs/heads/develop",
            COMMIT_CANDIDATE_HK_RECEIPT,
        );
        assert_eq!(
            scope,
            format!(
                "yo.slice-accept-effects/v1alpha2;slice=example;candidate={};squash=true;push=none;commit_verification=candidate_hk_receipt;close=true",
                "a".repeat(40)
            )
        );
    }

    // 후보 base가 integration HEAD와 같고 현재 host/toolchain에서 exact 후보 diff를
    // 선택한 hk receipt가 있을 때만 중복 Git hook을 생략합니다.
    #[test]
    fn fast_commit_requires_exact_current_hk_receipt() {
        let candidate = "a".repeat(40);
        let mut gate = slice_gate::ReadyGate {
            slice: "example".to_owned(),
            candidate_commit: candidate.clone(),
            diff_hash: "sha256:diff".to_owned(),
            validation: vec![slice_gate::ReadyValidation {
                name: "hk".to_owned(),
                argv: vec![
                    "hk".to_owned(),
                    "check".to_owned(),
                    "--check".to_owned(),
                    "--from-ref".to_owned(),
                    "base".to_owned(),
                    "--to-ref".to_owned(),
                    candidate,
                ],
                status: "passed".to_owned(),
                reused: false,
                current_reusable_context: true,
            }],
            review_count: 1,
            known_unverified_environments: Vec::new(),
            commit_trailers: Vec::new(),
        };
        assert_eq!(
            fast_commit_verification(&gate, "base", "base"),
            COMMIT_CANDIDATE_HK_RECEIPT
        );
        gate.validation[0].current_reusable_context = false;
        assert_eq!(
            fast_commit_verification(&gate, "base", "base"),
            COMMIT_GIT_HOOKS
        );
        gate.validation[0].current_reusable_context = true;
        assert_eq!(
            fast_commit_verification(&gate, "base", "advanced"),
            COMMIT_GIT_HOOKS
        );
        gate.validation[0].argv = vec!["hk".to_owned(), "check".to_owned()];
        assert_eq!(
            fast_commit_verification(&gate, "base", "base"),
            COMMIT_GIT_HOOKS
        );
    }
}
