//! Pure text projections of frontend-independent Journal records.

use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse, ActivityUpdate,
    AgentCommand, AgentEvent, ApprovalDecision, TranscriptRecord, TurnOutcome,
};

use super::RequestUnavailableReason;

pub(super) fn request_text(records: &[TranscriptRecord], anchor: Option<usize>) -> String {
    let Some(index) = anchor else {
        return request_unavailable_text(None, RequestUnavailableReason::NoAssociatedRequest);
    };
    let Some(record) = records.get(index) else {
        return request_unavailable_text(None, RequestUnavailableReason::NoAssociatedRequest);
    };
    match request_association(record) {
        Some(request) => format!(
            "Request diagnostic\n\
             status: unavailable\n\
             reason: {}\n\
             anchor: observed record #{} ({})\n\
             associated request: session={} turn={} activity={} request={}\n\
             observation boundary: semantic Session Journal only\n\
             exchange/revisions/attempts/redaction: unavailable\n\
             Request Audit detail is not wired in this Slice.",
            RequestUnavailableReason::RequestAuditDetailUnavailable.code(),
            index + 1,
            record_name(record),
            request.activity().session_id().get().get(),
            request.activity().turn_id().get().get(),
            request.activity().activity_id().get().get(),
            request.request_id().get().get(),
        ),
        None => request_unavailable_text(
            Some((index, record)),
            RequestUnavailableReason::NoAssociatedRequest,
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

fn request_unavailable_text(
    anchor: Option<(usize, &TranscriptRecord)>,
    reason: RequestUnavailableReason,
) -> String {
    let anchor = anchor.map_or_else(
        || "anchor: no Journal record is currently viewed".to_owned(),
        |(index, record)| {
            format!(
                "anchor: observed record #{} ({})",
                index + 1,
                record_name(record)
            )
        },
    );
    format!(
        "Request diagnostic\n\
         status: unavailable\n\
         reason: {}\n\
         {anchor}\n\
         observation boundary: semantic Session Journal only\n\
         no direct request correlation exists on this exact record\n\
         nearby records were not selected.",
        reason.code(),
    )
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
    match record {
        TranscriptRecord::CommandCommitted(command) => match command {
            AgentCommand::CreateSession { session_id } => {
                format!("{prefix}\nsession={}", session_id.get().get())
            },
            AgentCommand::StartTurn { turn, input } | AgentCommand::SteerTurn { turn, input } => {
                format!(
                    "{prefix}\nsession={} turn={}\ninput={:?}",
                    turn.session_id().get().get(),
                    turn.turn_id().get().get(),
                    input.as_str()
                )
            },
            AgentCommand::RespondToActivity { request, response } => format!(
                "{prefix}\nsession={} turn={} activity={} request={}\nresponse={}",
                request.activity().session_id().get().get(),
                request.activity().turn_id().get().get(),
                request.activity().activity_id().get().get(),
                request.request_id().get().get(),
                response_name(response)
            ),
            AgentCommand::InterruptTurn { turn } => format!(
                "{prefix}\nsession={} turn={}",
                turn.session_id().get().get(),
                turn.turn_id().get().get()
            ),
        },
        TranscriptRecord::EventCommitted(event) => match event {
            AgentEvent::SessionCreated { session_id } => {
                format!("{prefix}\nsession={}", session_id.get().get())
            },
            AgentEvent::TurnStarted { turn } => format!(
                "{prefix}\nsession={} turn={}",
                turn.session_id().get().get(),
                turn.turn_id().get().get()
            ),
            AgentEvent::ActivityStarted { activity, kind } => format!(
                "{prefix}\nsession={} turn={} activity={}\nkind={}",
                activity.session_id().get().get(),
                activity.turn_id().get().get(),
                activity.activity_id().get().get(),
                activity_kind_name(*kind)
            ),
            AgentEvent::ActivityUpdated { activity, update } => format!(
                "{prefix}\nsession={} turn={} activity={}\n{}",
                activity.session_id().get().get(),
                activity.turn_id().get().get(),
                activity.activity_id().get().get(),
                update_text(update)
            ),
            AgentEvent::ActivityFinished { activity, outcome } => format!(
                "{prefix}\nsession={} turn={} activity={}\noutcome={}",
                activity.session_id().get().get(),
                activity.turn_id().get().get(),
                activity.activity_id().get().get(),
                activity_outcome_name(outcome)
            ),
            AgentEvent::TurnFinished { turn, outcome } => format!(
                "{prefix}\nsession={} turn={}\noutcome={}",
                turn.session_id().get().get(),
                turn.turn_id().get().get(),
                turn_outcome_name(outcome)
            ),
        },
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
        },
        TranscriptRecord::EventCommitted(event) => match event {
            AgentEvent::SessionCreated { .. } => "event.session_created",
            AgentEvent::TurnStarted { .. } => "event.turn_started",
            AgentEvent::ActivityStarted { .. } => "event.activity_started",
            AgentEvent::ActivityUpdated { .. } => "event.activity_updated",
            AgentEvent::ActivityFinished { .. } => "event.activity_finished",
            AgentEvent::TurnFinished { .. } => "event.turn_finished",
        },
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
