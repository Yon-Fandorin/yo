use std::path::Path;

use yo_core::session_repository::{
    LocalSessionReader, SessionUsageReceipt, SessionUsageSource, StoredSession,
    StoredSessionReader, UsageValue as StoredUsageValue, read_stored_session,
};

use super::model::{ReviewTarget, UsageFields, UsageReceipt, UsageSearch, UsageValue};

const MAX_INSPECTED_SESSIONS: usize = 64;
const SELECTION_BASIS: &str = "newest_updated_session_then_latest_matching_receipt";

pub(super) fn latest_receipt(
    repository: Option<&str>,
    target: &ReviewTarget,
) -> (UsageSearch, Option<UsageReceipt>) {
    let Some(repository) = repository else {
        return (
            UsageSearch {
                state: "unknown",
                inspected_sessions: 0,
                truncated: false,
                selection_basis: SELECTION_BASIS,
                detail: Some("session_repository_path was not supplied".to_owned()),
            },
            None,
        );
    };
    let reader = match LocalSessionReader::open(Path::new(repository)) {
        Ok(reader) => reader,
        Err(error) => {
            return (
                UsageSearch {
                    state: "unknown",
                    inspected_sessions: 0,
                    truncated: false,
                    selection_basis: SELECTION_BASIS,
                    detail: Some(format!("cannot open the Session repository: {error}")),
                },
                None,
            );
        },
    };
    let sessions = match reader.discover() {
        Ok(sessions) => sessions,
        Err(error) => {
            return (
                UsageSearch {
                    state: "unknown",
                    inspected_sessions: 0,
                    truncated: false,
                    selection_basis: SELECTION_BASIS,
                    detail: Some(format!("cannot discover stored Sessions: {error}")),
                },
                None,
            );
        },
    };
    let truncated = sessions.len() > MAX_INSPECTED_SESSIONS;
    let mut inspected = 0;
    for session in sessions.into_iter().take(MAX_INSPECTED_SESSIONS) {
        inspected += 1;
        let session_id = session.session_id();
        if let Some(search) = unavailable_search(&session, inspected, truncated) {
            return (search, None);
        }
        let StoredSession::Available(summary) = session else {
            unreachable!("unavailable Sessions return before history projection");
        };
        let history = match read_stored_session(&reader, session_id) {
            Ok(history) => history,
            Err(error) => {
                return (
                    UsageSearch {
                        state: "unknown",
                        inspected_sessions: inspected,
                        truncated,
                        selection_basis: SELECTION_BASIS,
                        detail: Some(format!("cannot read a candidate Session: {error}")),
                    },
                    None,
                );
            },
        };
        let projection = match history.session_usage() {
            Ok(projection) => projection,
            Err(error) => {
                return (
                    UsageSearch {
                        state: "unknown",
                        inspected_sessions: inspected,
                        truncated,
                        selection_basis: SELECTION_BASIS,
                        detail: Some(format!("cannot project a candidate usage receipt: {error}")),
                    },
                    None,
                );
            },
        };
        if let Some(receipt) = projection
            .receipts()
            .iter()
            .rev()
            .find(|receipt| matches_target(receipt, target))
        {
            return (
                UsageSearch {
                    state: "reported",
                    inspected_sessions: inspected,
                    truncated,
                    selection_basis: SELECTION_BASIS,
                    detail: None,
                },
                Some(project_receipt(
                    session_id.to_string(),
                    summary.discovery().updated_unix_millis(),
                    receipt,
                )),
            );
        }
    }
    (
        UsageSearch {
            state: if truncated { "unknown" } else { "absent" },
            inspected_sessions: inspected,
            truncated,
            selection_basis: SELECTION_BASIS,
            detail: truncated.then(|| {
                format!(
                    "no matching receipt was found within the newest {MAX_INSPECTED_SESSIONS} Sessions"
                )
            }),
        },
        None,
    )
}

pub(super) fn unavailable_search(
    session: &StoredSession,
    inspected_sessions: usize,
    truncated: bool,
) -> Option<UsageSearch> {
    session.unavailable_reason().map(|reason| UsageSearch {
        state: "unknown",
        inspected_sessions,
        truncated,
        selection_basis: SELECTION_BASIS,
        detail: Some(format!(
            "cannot inspect candidate Session {}: {reason}",
            session.session_id()
        )),
    })
}

fn matches_target(receipt: &SessionUsageReceipt, target: &ReviewTarget) -> bool {
    match (receipt.source(), target) {
        (
            SessionUsageSource::Managed {
                provider,
                account,
                model,
                ..
            },
            ReviewTarget::ManagedModel {
                provider: expected_provider,
                account: expected_account,
                model: expected_model,
            },
        ) => {
            provider == expected_provider && account == expected_account && model == expected_model
        },
        (SessionUsageSource::Codex { .. }, ReviewTarget::DelegatedHost { host }) => host == "codex",
        (SessionUsageSource::Grok { .. }, ReviewTarget::DelegatedHost { host }) => host == "grok",
        _ => false,
    }
}

fn project_receipt(
    session_id: String,
    session_updated_unix_millis: u64,
    receipt: &SessionUsageReceipt,
) -> UsageReceipt {
    let source = match receipt.source() {
        SessionUsageSource::Managed {
            response_id,
            round,
            provider,
            account,
            model,
            connector,
            api_dialect,
            base_url,
        } => serde_json::json!({
            "response_id": response_id,
            "round": round,
            "provider": provider,
            "account": account,
            "model": model,
            "connector": connector,
            "api_dialect": api_dialect,
            "base_url": base_url,
        }),
        SessionUsageSource::Grok {
            source_profile,
            prompt_request_id,
        } => serde_json::json!({
            "source_profile": source_profile,
            "prompt_request_id": prompt_request_id,
        }),
        SessionUsageSource::Codex {
            source_profile,
            turn_id,
            model_context_window,
        } => serde_json::json!({
            "source_profile": source_profile,
            "turn_id": turn_id,
            "model_context_window": model_context_window,
        }),
    };
    let usage = receipt.usage();
    UsageReceipt {
        session_id,
        session_updated_unix_millis,
        schema: receipt.schema(),
        source,
        usage: UsageFields {
            input_tokens: usage_value(usage.input_tokens()),
            output_tokens: usage_value(usage.output_tokens()),
            total_tokens: usage_value(usage.total_tokens()),
            reasoning_tokens: usage_value(usage.reasoning_tokens()),
            cache_read_input_tokens: usage_value(usage.cache_read_input_tokens()),
            cache_write_input_tokens: usage_value(usage.cache_write_input_tokens()),
        },
    }
}

const fn usage_value(value: StoredUsageValue) -> UsageValue {
    match value {
        StoredUsageValue::Reported(tokens) => UsageValue::Reported { tokens },
        StoredUsageValue::Absent => UsageValue::Absent,
        StoredUsageValue::Unsupported => UsageValue::Unsupported,
    }
}
