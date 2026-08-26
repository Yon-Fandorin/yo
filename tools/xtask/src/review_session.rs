use serde_json::Value;

pub(crate) fn managed_binding_matches(
    binding_identity: &str,
    provider: &str,
    account: &str,
    model: &str,
) -> Result<bool, String> {
    let binding: Value = serde_json::from_str(binding_identity)
        .map_err(|error| format!("managed binding identity is not JSON: {error}"))?;
    Ok([
        ("provider", provider),
        ("account", account),
        ("model", model),
    ]
    .into_iter()
    .all(|(name, expected)| binding.get(name).and_then(Value::as_str) == Some(expected)))
}

pub(crate) fn delegated_binding_matches(
    binding_schema: &str,
    binding_identity: &str,
    host: &str,
    execution_profile: &str,
) -> Result<bool, String> {
    let expected_schema = match host {
        "codex" => "codex.app-server/thread-binding/v1alpha1",
        "grok" => "grok.acp/session-binding/v1alpha1",
        other => return Err(format!("unsupported delegated review host `{other}`")),
    };
    let binding: Value = serde_json::from_str(binding_identity)
        .map_err(|error| format!("delegated binding identity is not JSON: {error}"))?;
    Ok(binding_schema == expected_schema
        && binding.get("executionProfile").and_then(Value::as_str) == Some(execution_profile))
}

pub(crate) fn delegated_backend_kind_matches(backend_kind: &str, host: &str) -> bool {
    matches!(
        (backend_kind, host),
        ("codex-app-server", "codex") | ("grok-build-acp", "grok")
    )
}

pub(crate) fn provider_request_identity(
    request_identities: &[String],
    outcome_identities: &[Option<String>],
) -> Result<String, String> {
    if request_identities.len() != 1 || outcome_identities.len() != 1 {
        return Err(format!(
            "durable Session observed {} accepted requests and {} resumable outcomes; expected one each",
            request_identities.len(),
            outcome_identities.len()
        ));
    }
    Ok(outcome_identities[0]
        .clone()
        .unwrap_or_else(|| request_identities[0].clone()))
}

pub(crate) fn host_request_identity(
    request_identities: &[String],
    outcome_identities: &[Option<String>],
) -> Result<String, String> {
    if request_identities.len() != 1 || outcome_identities.len() != 1 {
        return Err(format!(
            "durable delegated Session observed {} accepted host requests and {} resumable outcomes; expected one each",
            request_identities.len(),
            outcome_identities.len()
        ));
    }
    Ok(outcome_identities[0]
        .clone()
        .unwrap_or_else(|| request_identities[0].clone()))
}

#[cfg(test)]
mod tests {
    use super::{
        delegated_backend_kind_matches, delegated_binding_matches, managed_binding_matches,
    };

    // 공유 route validator는 세 좌표가 모두 같은 managed identity만 허용하고 JSON이 아닌
    // opaque 값이나 한 좌표의 drift를 일치로 추측하지 않습니다.
    #[test]
    fn managed_binding_requires_the_exact_three_part_route() {
        let binding = r#"{"provider":"kimi","account":"default","model":"k3-256k"}"#;
        assert!(managed_binding_matches(binding, "kimi", "default", "k3-256k").unwrap());
        assert!(!managed_binding_matches(binding, "kimi", "other", "k3-256k").unwrap());
        assert!(managed_binding_matches("opaque", "kimi", "default", "k3-256k").is_err());
    }

    // delegated identity는 host별 alpha binding schema와 승인된 execution profile을 모두
    // 보존해야 하며, 표준 host Session이나 다른 host의 binding을 review로 승격하지 않습니다.
    #[test]
    fn delegated_binding_requires_host_and_review_profile() {
        let value =
            r#"{"executionProfile":"yo.delegated-review-execution/v1alpha1","sessionId":"a"}"#;
        assert!(
            delegated_binding_matches(
                "codex.app-server/thread-binding/v1alpha1",
                value,
                "codex",
                "yo.delegated-review-execution/v1alpha1"
            )
            .unwrap()
        );
        assert!(
            !delegated_binding_matches(
                "codex.app-server/thread-binding/v1",
                value,
                "codex",
                "yo.delegated-review-execution/v1alpha1"
            )
            .unwrap()
        );
        assert!(
            !delegated_binding_matches(
                "codex.app-server/thread-binding/v1alpha1",
                value,
                "grok",
                "yo.delegated-review-execution/v1alpha1"
            )
            .unwrap()
        );
    }

    // public host target와 backend durable kind는 서로 다른 identity 문자열이므로 exact
    // reviewed mapping만 허용하고 문자열 동등성이나 접두사 추측을 사용하지 않습니다.
    #[test]
    fn delegated_host_maps_to_the_exact_backend_kind() {
        assert!(delegated_backend_kind_matches("codex-app-server", "codex"));
        assert!(delegated_backend_kind_matches("grok-build-acp", "grok"));
        assert!(!delegated_backend_kind_matches("codex", "codex"));
        assert!(!delegated_backend_kind_matches("codex-app-server", "grok"));
    }
}
