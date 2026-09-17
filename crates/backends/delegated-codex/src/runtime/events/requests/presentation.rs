//! 승인 요청의 제한된 표시 문자열과 바이트 제한 writer.

use std::io::{self, Write};

use serde_json::Value;
use yo_core::{BackendFailure, ToolOutput};

use crate::protocol;

pub(super) fn approval_summary(method: &str, params: &Value) -> Result<String, BackendFailure> {
    let mut output = ApprovalText(Vec::new());
    let render = |output: &mut ApprovalText| -> Result<(), io::Error> {
        output.write_all(if method == "item/fileChange/requestApproval" {
            b"File change approval"
        } else {
            b"Command approval"
        })?;
        for (key, label) in [
            ("kind", "Action kind"),
            ("command", "Command"),
            ("cwd", "Working directory"),
            ("environmentId", "Environment"),
            (
                "grantRoot",
                if method == "item/fileChange/requestApproval" {
                    "Requested write root (session scope)"
                } else {
                    "Reported grantRoot"
                },
            ),
            ("reason", "Reason"),
            ("networkApprovalContext", "Network target"),
            ("additionalPermissions", "Additional permissions"),
            ("commandActions", "Reported command actions"),
            ("proposedExecpolicyAmendment", "Proposed command policy"),
            (
                "proposedNetworkPolicyAmendments",
                "Proposed network policies",
            ),
            ("availableDecisions", "Offered decisions"),
        ] {
            let Some(value) = params.get(key).filter(|value| !value.is_null()) else {
                continue;
            };
            output.write_all(b"\n")?;
            output.write_all(label.as_bytes())?;
            output.write_all(b": ")?;
            match value {
                Value::String(text) => output.write_all(text.as_bytes())?,
                value => serde_json::to_writer(&mut *output, value).map_err(io::Error::other)?,
            }
        }
        if let Some(fields) = params.as_object() {
            for (key, value) in fields.iter().filter(|(key, _)| {
                ![
                    "threadId",
                    "turnId",
                    "itemId",
                    "approvalId",
                    "startedAtMs",
                    "kind",
                    "command",
                    "cwd",
                    "environmentId",
                    "grantRoot",
                    "reason",
                    "networkApprovalContext",
                    "additionalPermissions",
                    "commandActions",
                    "proposedExecpolicyAmendment",
                    "proposedNetworkPolicyAmendments",
                    "availableDecisions",
                ]
                .contains(&key.as_str())
            }) {
                output.write_all(b"\nAdditional field ")?;
                serde_json::to_writer(&mut *output, key).map_err(io::Error::other)?;
                output.write_all(b": ")?;
                serde_json::to_writer(&mut *output, value).map_err(io::Error::other)?;
            }
        }
        Ok(())
    };
    render(&mut output)
        .map_err(|_| protocol::protocol_failure("approval context exceeds the display limit"))?;
    Ok(String::from_utf8(output.0).expect("approval fields and JSON are UTF-8"))
}

struct ApprovalText(Vec<u8>);

impl Write for ApprovalText {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|size| size > ToolOutput::MAX_SNAPSHOT_BYTES)
        {
            return Err(io::Error::other("approval display limit exceeded"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
