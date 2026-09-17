use yo_core::{
    session_repository as core_session_repository,
    session_repository::{SessionUsageSource, StoredRequestTraceRecord},
};

use super::model::{UsageBinding, UsageReceipt, UsageSource, UsageTarget};
use crate::review_session::{host_request_identity, provider_request_identity};

pub(super) fn require_request_binding(
    history: &core_session_repository::StoredSessionHistory,
    binding: &UsageBinding,
) -> Result<(), String> {
    let mut requests = Vec::new();
    let mut outcomes = Vec::new();
    for entry in history.request_trace() {
        match entry.record() {
            StoredRequestTraceRecord::RequestAccepted {
                turn_id,
                request_identity,
                ..
            } if *turn_id == binding.turn_id => {
                requests.push(request_identity.value().to_owned());
            },
            StoredRequestTraceRecord::ResumableOutcome {
                turn_id,
                outcome_identity,
                ..
            } if *turn_id == binding.turn_id => outcomes.push(
                outcome_identity
                    .as_ref()
                    .map(|identity| identity.value().to_owned()),
            ),
            _ => {},
        }
    }
    require_bound_request_identity(&requests, &outcomes, &binding.target, &binding.request_id)
}

pub(super) fn require_bound_request_identity(
    requests: &[String],
    outcomes: &[Option<String>],
    target: &UsageTarget,
    expected: &str,
) -> Result<(), String> {
    let observed = match target {
        UsageTarget::ManagedModel { .. } => provider_request_identity(requests, outcomes),
        UsageTarget::DelegatedHost { .. } => host_request_identity(requests, outcomes),
    }
    .map_err(|error| format!("usage binding has no exact request turn: {error}"))?;
    if observed == expected {
        Ok(())
    } else {
        Err(format!(
            "usage binding request identity mismatch: expected {}, found {observed}",
            expected
        ))
    }
}

pub(super) fn source_matches(source: &SessionUsageSource, target: &UsageTarget) -> bool {
    match (source, target) {
        (
            SessionUsageSource::Managed {
                provider,
                account,
                model,
                ..
            },
            UsageTarget::ManagedModel {
                provider: expected_provider,
                account: expected_account,
                model: expected_model,
            },
        ) => {
            provider == expected_provider && account == expected_account && model == expected_model
        },
        (
            SessionUsageSource::Grok { .. } | SessionUsageSource::GrokDiagnostic { .. },
            UsageTarget::DelegatedHost { host },
        ) => host == "grok",
        (SessionUsageSource::Codex { .. }, UsageTarget::DelegatedHost { host }) => host == "codex",
        _ => false,
    }
}

pub(super) fn require_source_request_binding(
    receipts: &[UsageReceipt],
    target: &UsageTarget,
    request_id: &str,
) -> Result<(), String> {
    if receipts.is_empty() {
        return Ok(());
    }
    let matches = match target {
        UsageTarget::ManagedModel { .. } => matches!(
            receipts.last().map(|receipt| &receipt.source),
            Some(UsageSource::Managed { response_id, .. }) if response_id == request_id
        ),
        UsageTarget::DelegatedHost { host } if host == "grok" => {
            let identity: serde_json::Value = serde_json::from_str(request_id)
                .map_err(|error| format!("Grok usage request identity is not JSON: {error}"))?;
            let expected = identity
                .get("jsonRpcId")
                .and_then(serde_json::Value::as_u64);
            expected.is_some_and(|expected| {
                receipts.iter().all(|receipt| {
                    matches!(
                        receipt.source,
                        UsageSource::Grok {
                            prompt_request_id,
                            ..
                        } if prompt_request_id == expected
                    )
                })
            })
        },
        UsageTarget::DelegatedHost { host } if host == "codex" => {
            let identity: serde_json::Value = serde_json::from_str(request_id)
                .map_err(|error| format!("Codex usage request identity is not JSON: {error}"))?;
            let expected = identity.get("turnId").and_then(serde_json::Value::as_str);
            expected.is_some_and(|expected| {
                receipts.iter().all(|receipt| {
                    matches!(
                        &receipt.source,
                        UsageSource::Codex { turn_id, .. } if turn_id == expected
                    )
                })
            })
        },
        UsageTarget::DelegatedHost { .. } => false,
    };
    if matches {
        Ok(())
    } else {
        Err("usage source identity does not match the exact delivery request".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use yo_core::session_repository::SessionUsageSource;

    use super::{UsageTarget, require_bound_request_identity, source_matches};

    // usage artifact는 같은 turn에서 관측한 exact outcome identity만 받으며, 다른 identity나
    // 중복 request를 가장 가까운 값으로 추측하지 않습니다.
    #[test]
    fn usage_binding_requires_one_exact_external_request_identity() {
        let target = UsageTarget::ManagedModel {
            provider: "kimi".to_owned(),
            account: "default".to_owned(),
            model: "k3".to_owned(),
        };
        let requests = ["request-1".to_owned()];
        let outcomes = [Some("response-1".to_owned())];
        assert!(
            require_bound_request_identity(&requests, &outcomes, &target, "response-1").is_ok()
        );
        assert!(
            require_bound_request_identity(&requests, &outcomes, &target, "request-1").is_err()
        );
        assert!(
            require_bound_request_identity(
                &["request-1".to_owned(), "request-2".to_owned()],
                &outcomes,
                &target,
                "response-1",
            )
            .is_err()
        );
    }

    // Kimi/Qwen managed route와 Grok/Codex delegated route는 각각 자기 source variant와
    // exact 좌표에만 대응하며 다른 Provider나 host의 영수증을 재사용하지 않습니다.
    #[test]
    fn usage_source_matching_covers_every_external_review_route() {
        let managed = SessionUsageSource::Managed {
            response_id: "response-1".to_owned(),
            round: 1,
            provider: "kimi".to_owned(),
            account: "default".to_owned(),
            model: "k3".to_owned(),
            connector: "kimi".to_owned(),
            api_dialect: "chat-completions".to_owned(),
            base_url: "https://example.invalid".to_owned(),
        };
        assert!(source_matches(
            &managed,
            &UsageTarget::ManagedModel {
                provider: "kimi".to_owned(),
                account: "default".to_owned(),
                model: "k3".to_owned(),
            }
        ));
        assert!(!source_matches(
            &managed,
            &UsageTarget::ManagedModel {
                provider: "qwencloud".to_owned(),
                account: "default".to_owned(),
                model: "qwen3.8-max".to_owned(),
            }
        ));

        let grok = SessionUsageSource::Grok {
            source_profile: "grok.acp.prompt-response.usage/v1".to_owned(),
            prompt_request_id: 7,
        };
        let codex = SessionUsageSource::Codex {
            source_profile: "codex.app-server.thread-token-usage-updated/v1".to_owned(),
            turn_id: "turn-1".to_owned(),
            model_context_window: Some(258_000),
        };
        assert!(source_matches(
            &grok,
            &UsageTarget::DelegatedHost {
                host: "grok".to_owned(),
            }
        ));
        assert!(source_matches(
            &codex,
            &UsageTarget::DelegatedHost {
                host: "codex".to_owned(),
            }
        ));
        assert!(!source_matches(
            &codex,
            &UsageTarget::DelegatedHost {
                host: "grok".to_owned(),
            }
        ));
    }
}
