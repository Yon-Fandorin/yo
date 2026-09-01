//! Pure text projections of frontend-independent Journal records.

use unicode_segmentation::UnicodeSegmentation;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse, ActivityUpdate,
    AgentCommand, AgentEvent, ApprovalDecision, RequestTraceEntry, TranscriptRecord, TurnOutcome,
};

use super::super::archival::ArchivedContentPolicy;
#[cfg(test)]
use super::RequestUnavailableReason;

const ARCHIVED_CONTENT_PREVIEW_BYTES: usize = 256;

pub(super) fn request_text(
    records: &[TranscriptRecord],
    anchor: Option<usize>,
    trace: &[RequestTraceEntry],
) -> String {
    let context = request_context_text(records, anchor);
    let records = if trace.is_empty() {
        "no correlation records have been committed".to_owned()
    } else {
        trace
            .iter()
            .map(|entry| {
                super::super::archival::request::format_record(
                    entry.sequence().get(),
                    entry.record(),
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    format!(
        "Live Session Request diagnostic\n\
         {context}\n\
         observation_boundary=committed-live-session-journal\n\
         request_audit_detail=unavailable(reason=no-audit-reader)\n\
         \n\
         {records}"
    )
}

fn request_context_text(records: &[TranscriptRecord], anchor: Option<usize>) -> String {
    let Some(index) = anchor else {
        return "context_highlight=none(reason=no-viewed-journal-record)".to_owned();
    };
    let Some(record) = records.get(index) else {
        return "context_highlight=none(reason=no-viewed-journal-record)".to_owned();
    };
    match request_association(record) {
        Some(request) => format!(
            "context_highlight=direct-activity-request activity={} request={}\n\
             context_record={} session={} turn={}",
            request.activity().activity_id().get().get(),
            request.request_id().get().get(),
            index + 1,
            request.activity().session_id(),
            request.activity().turn_id().get().get(),
        ),
        None => format!(
            "context_highlight=none(reason=no-direct-request)\ncontext_record={} ({})",
            index + 1,
            record_name(record)
        ),
    }
}

#[cfg(test)]
pub(super) fn request_reason(
    records: &[TranscriptRecord],
    anchor: Option<usize>,
) -> RequestUnavailableReason {
    anchor
        .and_then(|index| records.get(index))
        .and_then(request_association)
        .map_or(RequestUnavailableReason::NoAssociatedRequest, |_| {
            RequestUnavailableReason::RequestAuditDetailUnavailable
        })
}

fn request_association(record: &TranscriptRecord) -> Option<ActivityRequestRef> {
    match record {
        TranscriptRecord::CommandCommitted(AgentCommand::RespondToActivity { request, .. }) => {
            Some(*request)
        },
        TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted {
            activity,
            kind:
                ActivityKind::ApprovalRequest { request_id }
                | ActivityKind::ApprovalResponse { request_id }
                | ActivityKind::UserInputRequest { request_id }
                | ActivityKind::UserInputResponse { request_id },
        }) => Some(ActivityRequestRef::new(*activity, *request_id)),
        _ => None,
    }
}

pub(super) fn format_record(index: usize, record: &TranscriptRecord) -> String {
    let record_prefix = format!("[#{:03}] {}", index + 1, record_name(record));
    let prefix = if index == 0 {
        format!(
            "[observation boundary] semantic records only; JournalSequence, durability-gap \
             metadata, and Request Audit detail are unavailable at the current TUI reader seam.\n\
             {record_prefix}"
        )
    } else {
        record_prefix
    };
    format_record_body(prefix, record)
}

pub(in crate::runner) fn format_archival_record(
    index: usize,
    record: &TranscriptRecord,
    content: ArchivedContentPolicy,
) -> String {
    let prefix = format!("[#{:03}] {}", index + 1, record_name(record));
    if content == ArchivedContentPolicy::Full {
        return format_record_body(prefix, record);
    }
    format_bounded_archival_record(prefix, record, content)
}

fn format_bounded_archival_record(
    prefix: String,
    record: &TranscriptRecord,
    content: ArchivedContentPolicy,
) -> String {
    match record {
        TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn { turn, input } | AgentCommand::SteerTurn { turn, input },
        ) => format!(
            "{prefix}\nsession={} turn={}\n{}",
            turn.session_id(),
            turn.turn_id().get().get(),
            bounded_content("user_input", input.as_str(), content)
        ),
        TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { activity, update }) => {
            let (content_type, text) = match update {
                ActivityUpdate::TextDelta(text) => ("activity_text_delta", text.as_str()),
                ActivityUpdate::TextSnapshot(text) => ("activity_text_snapshot", text.as_str()),
            };
            format!(
                "{prefix}\nsession={} turn={} activity={}\n{}",
                activity.session_id(),
                activity.turn_id().get().get(),
                activity.activity_id().get().get(),
                bounded_content(content_type, text, content)
            )
        },
        TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Failed(failure),
        }) => format!(
            "{prefix}\nsession={} turn={} activity={}\noutcome=failed\n{}",
            activity.session_id(),
            activity.turn_id().get().get(),
            activity.activity_id().get().get(),
            bounded_content("activity_failure_message", failure.message(), content)
        ),
        TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Failed(failure),
        }) => format!(
            "{prefix}\nsession={} turn={}\noutcome=failed\n{}",
            turn.session_id(),
            turn.turn_id().get().get(),
            bounded_content("turn_failure_message", failure.message(), content)
        ),
        _ => format_record_body(prefix, record),
    }
}

fn bounded_content(content_type: &str, value: &str, policy: ArchivedContentPolicy) -> String {
    let metadata = format!(
        "content.type={content_type}\ncontent.utf8_bytes={}",
        value.len()
    );
    match policy {
        ArchivedContentPolicy::None => metadata,
        ArchivedContentPolicy::Preview => {
            let (preview, truncated) = utf8_bounded_preview(value);
            format!(
                "{metadata}\ncontent.preview={preview:?}\ncontent.preview_truncated={truncated}"
            )
        },
        ArchivedContentPolicy::Full => {
            unreachable!("full archived content uses the byte-compatible legacy formatter")
        },
    }
}

fn utf8_bounded_preview(value: &str) -> (&str, bool) {
    if value.len() <= ARCHIVED_CONTENT_PREVIEW_BYTES {
        return (value, false);
    }
    let end = value
        .grapheme_indices(true)
        .map(|(start, grapheme)| start + grapheme.len())
        .take_while(|end| *end <= ARCHIVED_CONTENT_PREVIEW_BYTES)
        .last()
        .unwrap_or(0);
    (&value[..end], true)
}

fn format_record_body(prefix: String, record: &TranscriptRecord) -> String {
    match record {
        TranscriptRecord::CommandCommitted(command) => match command {
            AgentCommand::CreateSession { session_id } => {
                format!("{prefix}\nsession={session_id}")
            },
            AgentCommand::StartTurn { turn, input } | AgentCommand::SteerTurn { turn, input } => {
                format!(
                    "{prefix}\nsession={} turn={}\ninput={:?}",
                    turn.session_id(),
                    turn.turn_id().get().get(),
                    input.as_str()
                )
            },
            AgentCommand::RespondToActivity { request, response } => format!(
                "{prefix}\nsession={} turn={} activity={} request={}\nresponse={}",
                request.activity().session_id(),
                request.activity().turn_id().get().get(),
                request.activity().activity_id().get().get(),
                request.request_id().get().get(),
                response_name(response)
            ),
            AgentCommand::InterruptTurn { turn } => format!(
                "{prefix}\nsession={} turn={}",
                turn.session_id(),
                turn.turn_id().get().get()
            ),
            AgentCommand::CompactContext { guidance } => {
                format!("{prefix}\nguidance={guidance:?}")
            },
        },
        TranscriptRecord::EventCommitted(event) => match event {
            AgentEvent::SessionCreated { session_id } => {
                format!("{prefix}\nsession={session_id}")
            },
            AgentEvent::TurnStarted { turn } => format!(
                "{prefix}\nsession={} turn={}",
                turn.session_id(),
                turn.turn_id().get().get()
            ),
            AgentEvent::ActivityStarted { activity, kind } => format!(
                "{prefix}\nsession={} turn={} activity={}\nkind={}",
                activity.session_id(),
                activity.turn_id().get().get(),
                activity.activity_id().get().get(),
                activity_kind_name(*kind)
            ),
            AgentEvent::ActivityUpdated { activity, update } => format!(
                "{prefix}\nsession={} turn={} activity={}\n{}",
                activity.session_id(),
                activity.turn_id().get().get(),
                activity.activity_id().get().get(),
                update_text(update)
            ),
            AgentEvent::ActivityFinished { activity, outcome } => format!(
                "{prefix}\nsession={} turn={} activity={}\noutcome={}",
                activity.session_id(),
                activity.turn_id().get().get(),
                activity.activity_id().get().get(),
                activity_outcome_name(outcome)
            ),
            AgentEvent::TurnFinished { turn, outcome } => format!(
                "{prefix}\nsession={} turn={}\noutcome={}",
                turn.session_id(),
                turn.turn_id().get().get(),
                turn_outcome_name(outcome)
            ),
        },
        TranscriptRecord::ContextPolicyChanged(policy) => format!(
            "{prefix}\nrevision={} enabled={} strategy={} warning_percent={} trigger_percent={} retained_raw_percent={:?} retained_raw_max_tokens={:?}",
            policy.policy_revision(),
            policy.enabled(),
            policy.strategy().as_str(),
            policy.warning_percent(),
            policy.trigger_percent(),
            policy.retained_raw_percent(),
            policy.retained_raw_max_tokens(),
        ),
        TranscriptRecord::ContextCheckpointCommitted(checkpoint) => format!(
            "{prefix}\nsource={}..{} policy_revision={} context_epoch={}..{} input_tokens={}..{} input_token_limit={} retained_groups={} artifact_receipts={} visible_prefix_losses={} provider_private_losses={}",
            checkpoint.source_anchor_sequence().get(),
            checkpoint.source_journal_boundary().get(),
            checkpoint.policy_revision(),
            checkpoint.previous_context_epoch(),
            checkpoint.successor_context_epoch(),
            checkpoint.input_tokens_before(),
            checkpoint.input_tokens_after(),
            checkpoint.input_token_limit(),
            checkpoint.retained_group_count(),
            checkpoint.artifact_receipt_count(),
            checkpoint.visible_prefix_loss_count(),
            checkpoint.provider_private_loss_count(),
        ),
    }
}

fn record_name(record: &TranscriptRecord) -> &'static str {
    match record {
        TranscriptRecord::CommandCommitted(command) => match command {
            AgentCommand::CreateSession { .. } => "command.create_session",
            AgentCommand::StartTurn { .. } => "command.start_turn",
            AgentCommand::SteerTurn { .. } => "command.steer_turn",
            AgentCommand::RespondToActivity { .. } => "command.respond_to_activity",
            AgentCommand::InterruptTurn { .. } => "command.interrupt_turn",
            AgentCommand::CompactContext { .. } => "command.compact_context",
        },
        TranscriptRecord::EventCommitted(event) => match event {
            AgentEvent::SessionCreated { .. } => "event.session_created",
            AgentEvent::TurnStarted { .. } => "event.turn_started",
            AgentEvent::ActivityStarted { .. } => "event.activity_started",
            AgentEvent::ActivityUpdated { .. } => "event.activity_updated",
            AgentEvent::ActivityFinished { .. } => "event.activity_finished",
            AgentEvent::TurnFinished { .. } => "event.turn_finished",
        },
        TranscriptRecord::ContextPolicyChanged(_) => "context.policy_changed",
        TranscriptRecord::ContextCheckpointCommitted(_) => "context.checkpoint_committed",
    }
}

fn response_name(response: &ActivityResponse) -> &'static str {
    match response {
        ActivityResponse::Approval(ApprovalDecision::Approved) => "approval.approved",
        ActivityResponse::Approval(ApprovalDecision::Declined) => "approval.declined",
        ActivityResponse::UserInput(_) => "user_input",
    }
}

fn activity_kind_name(kind: ActivityKind) -> &'static str {
    match kind {
        ActivityKind::ModelWork => "model_work",
        ActivityKind::AgentMessage => "agent_message",
        ActivityKind::ToolCall => "tool_call",
        ActivityKind::ToolResult => "tool_result",
        ActivityKind::FileChange => "file_change",
        ActivityKind::ApprovalRequest { .. } => "approval_request",
        ActivityKind::ApprovalResponse { .. } => "approval_response",
        ActivityKind::UserInputRequest { .. } => "user_input_request",
        ActivityKind::UserInputResponse { .. } => "user_input_response",
    }
}

fn update_text(update: &ActivityUpdate) -> String {
    match update {
        ActivityUpdate::TextDelta(text) => format!("update=text_delta\ntext={text:?}"),
        ActivityUpdate::TextSnapshot(text) => format!("update=text_snapshot\ntext={text:?}"),
    }
}

fn activity_outcome_name(outcome: &ActivityOutcome) -> String {
    match outcome {
        ActivityOutcome::Completed => "completed".to_owned(),
        ActivityOutcome::Interrupted => "interrupted".to_owned(),
        ActivityOutcome::Failed(failure) => format!("failed ({})", failure.message()),
    }
}

fn turn_outcome_name(outcome: &TurnOutcome) -> String {
    match outcome {
        TurnOutcome::Completed => "completed".to_owned(),
        TurnOutcome::Interrupted => "interrupted".to_owned(),
        TurnOutcome::Failed(failure) => format!("failed ({})", failure.message()),
    }
}
