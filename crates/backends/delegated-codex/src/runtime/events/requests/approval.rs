//! Codex 승인 요청의 제안 결정과 UI 선택지를 투영합니다.

use serde_json::{Value, json};
use yo_core::{ApprovalChoice, BackendFailure};

use crate::protocol;

// Codex e1eb98461cd4의 tui/approval_events.rs::default_available_decisions와
// bottom_pane/approval_overlay.rs::patch_options를 반영합니다. 서버가 명시한 목록이
// 있으면 항상 그 목록을 우선합니다.
pub(super) fn approval_decisions(
    method: &str,
    params: &Value,
) -> Result<(Vec<Value>, bool), BackendFailure> {
    match params.get("availableDecisions") {
        Some(Value::Array(choices)) => return Ok((choices.clone(), true)),
        None | Some(Value::Null) => {},
        Some(_) => {
            return Err(protocol::protocol_failure(
                "approval availableDecisions must be an array or null",
            ));
        },
    }
    if method == "item/fileChange/requestApproval" {
        return Ok((
            vec![json!("accept"), json!("acceptForSession"), json!("cancel")],
            false,
        ));
    }
    let present = |key| params.get(key).filter(|value| !value.is_null());
    let mut decisions = vec![json!("accept")];
    if let Some(network) = present("networkApprovalContext") {
        if network.get("host").and_then(Value::as_str).is_none()
            || !matches!(
                network.get("protocol").and_then(Value::as_str),
                Some("http" | "https" | "socks5Tcp" | "socks5Udp")
            )
        {
            return Err(protocol::protocol_failure(
                "invalid network approval context",
            ));
        }
        decisions.push(json!("acceptForSession"));
        if let Some(proposals) = present("proposedNetworkPolicyAmendments") {
            let proposals = proposals
                .as_array()
                .ok_or_else(|| protocol::protocol_failure("invalid proposed network policies"))?;
            let mut first_allow = None;
            for proposal in proposals {
                let decision =
                    json!({"applyNetworkPolicyAmendment":{"network_policy_amendment":proposal}});
                if approval_choice(&decision, true).is_none() {
                    return Err(protocol::protocol_failure(
                        "invalid proposed network policy",
                    ));
                }
                if first_allow.is_none()
                    && proposal.get("action").and_then(Value::as_str) == Some("allow")
                {
                    first_allow = Some(decision);
                }
            }
            decisions.extend(first_allow);
        }
    } else if let Some(permissions) = present("additionalPermissions") {
        if !permissions.is_object() {
            return Err(protocol::protocol_failure(
                "invalid additional approval permissions",
            ));
        }
    } else if let Some(prefix) = present("proposedExecpolicyAmendment") {
        let decision = json!({"acceptWithExecpolicyAmendment":{"execpolicy_amendment":prefix}});
        if approval_choice(&decision, true).is_none() {
            return Err(protocol::protocol_failure(
                "invalid proposed command policy",
            ));
        }
        decisions.push(decision);
    }
    decisions.push(json!("cancel"));
    Ok((decisions, false))
}

pub(in crate::runtime::events) fn approval_choice(
    value: &Value,
    command: bool,
) -> Option<ApprovalChoice> {
    if !command && !value.is_string() {
        return None;
    }
    let (label, description) = match value.as_str() {
        Some("accept") => (
            "Approve request",
            "Use the scope described in this request.".to_owned(),
        ),
        Some("acceptForSession") => (
            "Approve for session",
            "Future matching prompts in this session may run without asking again.".to_owned(),
        ),
        Some("decline") => (
            "Decline",
            "Do not run this action; continue the turn.".to_owned(),
        ),
        Some("cancel") => (
            "Decline and stop",
            "Do not run this action; interrupt the turn.".to_owned(),
        ),
        Some(_) => return None,
        None => {
            let fields = value.as_object()?;
            if fields.len() != 1 {
                return None;
            }
            if let Some(amendment) = fields.get("acceptWithExecpolicyAmendment") {
                let fields = amendment.as_object()?;
                if fields.len() != 1 {
                    return None;
                }
                let prefix = fields.get("execpolicy_amendment")?.as_array()?;
                if !prefix.iter().all(Value::is_string) {
                    return None;
                }
                (
                    "Approve + save rule",
                    format!(
                        "Persistent rule: future matching commands may run without prompting. Prefix: {}",
                        serde_json::to_string(prefix).ok()?
                    ),
                )
            } else {
                let fields = fields.get("applyNetworkPolicyAmendment")?.as_object()?;
                if fields.len() != 1 {
                    return None;
                }
                let rule = fields.get("network_policy_amendment")?.as_object()?;
                if rule.len() != 2 {
                    return None;
                }
                let host = rule.get("host")?.as_str()?;
                match rule.get("action")?.as_str()? {
                    "allow" => (
                        "Always allow host",
                        format!("Persistent network allow rule for host: {host}"),
                    ),
                    "deny" => (
                        "Always deny host",
                        format!("Persistent network deny rule for host: {host}"),
                    ),
                    _ => return None,
                }
            }
        },
    };
    Some(ApprovalChoice {
        label: label.to_owned(),
        description,
        enabled: true,
    })
}
