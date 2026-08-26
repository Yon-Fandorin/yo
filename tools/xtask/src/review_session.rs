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

#[cfg(test)]
mod tests {
    use super::managed_binding_matches;

    // 공유 route validator는 세 좌표가 모두 같은 managed identity만 허용하고 JSON이 아닌
    // opaque 값이나 한 좌표의 drift를 일치로 추측하지 않습니다.
    #[test]
    fn managed_binding_requires_the_exact_three_part_route() {
        let binding = r#"{"provider":"kimi","account":"default","model":"k3-256k"}"#;
        assert!(managed_binding_matches(binding, "kimi", "default", "k3-256k").unwrap());
        assert!(!managed_binding_matches(binding, "kimi", "other", "k3-256k").unwrap());
        assert!(managed_binding_matches("opaque", "kimi", "default", "k3-256k").is_err());
    }
}
