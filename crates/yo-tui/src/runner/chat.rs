//! Shared concise Chat projection for live and archived Session records.

use std::collections::HashMap;

use yo_core::{
    ActivityApproval, ActivityDocument, ActivityKind, ActivityNotice, ActivityOutcome,
    ActivityPlan, ActivityQuestion, ActivityReasoning, ActivityRef, ActivitySummary,
    ActivityUpdate, AgentCommand, AgentEvent, ContextCheckpointObservation, ContextPolicyChanged,
    ContextPressureDecision, ContextPressureObservation, SessionUsageProjection,
    SessionUsageSource, ToolOutput, TranscriptRecord, TurnOutcome, UsageValue,
    session_repository::InheritedSessionHistory,
};

use super::{TuiDocument, archival, state::StateError};
use crate::transcript::{
    TranscriptActivityOutcome, TranscriptBody, TranscriptItemId, TranscriptPhase, TranscriptState,
};

#[derive(Debug, Default)]
pub(super) struct ChatProjection {
    transcript: TranscriptState,
    publication_cursor: PublicationCursor,
    next_item_id: u64,
    activities: HashMap<ActivityRef, ActivityPresentation>,
    retained_changes: HashMap<ActivityRef, TranscriptItemId>,
    context_policy: Option<ContextPolicyChanged>,
    latest_usage: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PublicationCursor {
    boundary: Option<PublicationBoundary>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PublicationBoundary {
    item: TranscriptItemId,
    final_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PublicationCandidate {
    expected: PublicationCursor,
    range: std::ops::Range<usize>,
    boundary: PublicationBoundary,
}

impl PublicationCandidate {
    pub(super) fn range(&self) -> std::ops::Range<usize> {
        self.range.clone()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActivityPresentation {
    item: TranscriptItemId,
    kind: ActivityKind,
    has_payload: bool,
    visible: bool,
    usage: Option<String>,
    is_structured: bool,
    question: Option<ActivityQuestion>,
    approval: Option<ActivityApproval>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChatProjectionChange {
    Unchanged,
    VisibleItem(TranscriptItemId),
}

impl ChatProjection {
    pub(super) fn approval(&self, activity: ActivityRef) -> Option<&ActivityApproval> {
        self.activities.get(&activity)?.approval.as_ref()
    }

    pub(super) fn approval_change(&self, request: ActivityRef) -> Option<TranscriptItemId> {
        let id = self.approval(request)?.related_change?;
        self.activities
            .iter()
            .find_map(|(activity, presentation)| {
                (activity.turn() == request.turn()
                    && activity.activity_id().get().get() == id
                    && presentation.kind == ActivityKind::FileChange
                    && presentation.has_payload)
                    .then_some(presentation.item)
            })
            .or_else(|| {
                self.retained_changes.iter().find_map(|(activity, item)| {
                    (activity.turn() == request.turn() && activity.activity_id().get().get() == id)
                        .then_some(*item)
                })
            })
    }

    pub(super) fn question(&self, activity: ActivityRef) -> Option<&ActivityQuestion> {
        self.activities.get(&activity)?.question.as_ref()
    }

    pub(super) fn new() -> Self {
        Self {
            next_item_id: 1,
            ..Self::default()
        }
    }

    pub(super) fn transcript(&self) -> &TranscriptState {
        &self.transcript
    }

    pub(super) fn observe_inherited_history(
        &mut self,
        history: &InheritedSessionHistory,
    ) -> Result<(), StateError> {
        self.push_notice(archival::inherited_header(history))?;
        for section in history.sections() {
            self.push_notice(archival::inherited_section_header(section))?;
            self.observe_inherited_records(section.records())?;
        }
        self.push_notice("Current Session · live history".to_owned())?;
        Ok(())
    }

    fn observe_inherited_records(
        &mut self,
        records: &[TranscriptRecord],
    ) -> Result<(), StateError> {
        let mut source = Self::new();
        for record in records {
            source.observe_record(record)?;
        }
        for item in source.transcript.items() {
            let id = self.next_transcript_id()?;
            self.transcript
                .push_final_copy(id, item)
                .map_err(StateError::Transcript)?;
        }
        Ok(())
    }

    pub(super) fn latest_usage(&self) -> Option<&str> {
        self.latest_usage.as_deref()
    }

    pub(super) fn publication_candidate(&self) -> Option<PublicationCandidate> {
        let start = self.published_item_count();
        let end = self.transcript.items()[start..]
            .iter()
            .take_while(|item| item.phase() == TranscriptPhase::Final)
            .count()
            + start;
        if end == start {
            return None;
        }
        let last = self.transcript.items().get(end.checked_sub(1)?)?;
        Some(PublicationCandidate {
            expected: self.publication_cursor,
            range: start..end,
            boundary: PublicationBoundary {
                item: last.id(),
                final_revision: last.revision(),
            },
        })
    }

    pub(super) fn published_item_count(&self) -> usize {
        let Some(boundary) = self.publication_cursor.boundary else {
            return 0;
        };
        self.transcript
            .items()
            .iter()
            .position(|item| {
                item.id() == boundary.item
                    && item.revision() == boundary.final_revision
                    && item.phase() == TranscriptPhase::Final
            })
            .map(|index| index + 1)
            .expect("the publication cursor must name an immutable Final transcript item")
    }

    pub(super) fn acknowledge_publication(&mut self, candidate: &PublicationCandidate) -> bool {
        if self.publication_cursor != candidate.expected {
            return false;
        }
        let Some(last) = self
            .transcript
            .items()
            .get(candidate.range.end.saturating_sub(1))
        else {
            return false;
        };
        if last.id() != candidate.boundary.item
            || last.revision() != candidate.boundary.final_revision
            || last.phase() != TranscriptPhase::Final
        {
            return false;
        }
        self.publication_cursor.boundary = Some(candidate.boundary);
        true
    }

    pub(super) fn observe_record(
        &mut self,
        record: &TranscriptRecord,
    ) -> Result<ChatProjectionChange, StateError> {
        match record {
            TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn { input, .. } | AgentCommand::SteerTurn { input, .. },
            ) => {
                let id = self.next_transcript_id()?;
                self.transcript
                    .push_user(id, input.as_str().to_owned())
                    .map_err(StateError::Transcript)?;
                Ok(ChatProjectionChange::VisibleItem(id))
            },
            TranscriptRecord::CommandCommitted(_) => Ok(ChatProjectionChange::Unchanged),
            TranscriptRecord::EventCommitted(event) => self.observe_event(event),
            TranscriptRecord::ContextPolicyChanged(policy) => {
                self.context_policy = Some(policy.clone());
                Ok(ChatProjectionChange::Unchanged)
            },
            TranscriptRecord::ContextCheckpointCommitted(checkpoint) => {
                let id = self.push_notice(format_context_checkpoint(
                    checkpoint,
                    self.context_policy.as_ref(),
                ))?;
                Ok(ChatProjectionChange::VisibleItem(id))
            },
        }
    }

    fn observe_event(&mut self, event: &AgentEvent) -> Result<ChatProjectionChange, StateError> {
        match event {
            AgentEvent::SessionCreated { .. } | AgentEvent::TurnStarted { .. } => {
                Ok(ChatProjectionChange::Unchanged)
            },
            AgentEvent::ActivityStarted { activity, kind } => {
                let (item, visible) = self.start_activity(*activity, *kind)?;
                Ok(if visible {
                    ChatProjectionChange::VisibleItem(item)
                } else {
                    ChatProjectionChange::Unchanged
                })
            },
            AgentEvent::ActivityUpdated { activity, update } => {
                self.update_activity(*activity, update)
            },
            AgentEvent::ActivityFinished { activity, outcome } => {
                let (item, visible) = self.finish_activity(*activity, outcome)?;
                Ok(if visible {
                    ChatProjectionChange::VisibleItem(item)
                } else {
                    ChatProjectionChange::Unchanged
                })
            },
            AgentEvent::TurnFinished { outcome, .. } => match outcome {
                TurnOutcome::Completed => Ok(ChatProjectionChange::Unchanged),
                TurnOutcome::Interrupted => self
                    .push_outcome_notice(
                        "Turn interrupted".to_owned(),
                        TranscriptActivityOutcome::Interrupted,
                    )
                    .map(ChatProjectionChange::VisibleItem),
                TurnOutcome::Failed(failure) => self
                    .push_outcome_notice(
                        format!("Turn failed: {}", failure.message()),
                        TranscriptActivityOutcome::Failed,
                    )
                    .map(ChatProjectionChange::VisibleItem),
            },
        }
    }

    fn update_activity(
        &mut self,
        activity: ActivityRef,
        update: &ActivityUpdate,
    ) -> Result<ChatProjectionChange, StateError> {
        let Some(presentation) = self.activities.get_mut(&activity) else {
            return Err(StateError::UnknownActivity(activity));
        };
        match update {
            ActivityUpdate::TextDelta(text) => {
                presentation.usage = None;
                if text.is_empty() {
                    return Ok(ChatProjectionChange::Unchanged);
                }
                presentation.is_structured = false;
                presentation.question = None;
                presentation.approval = None;
                if !presentation.has_payload && activity_label(presentation.kind).is_some() {
                    self.transcript
                        .append_text(presentation.item, "\n")
                        .map_err(StateError::Transcript)?;
                }
                self.transcript
                    .append_text(presentation.item, text)
                    .map_err(StateError::Transcript)?;
                presentation.has_payload = true;
                presentation.visible = true;
                Ok(ChatProjectionChange::VisibleItem(presentation.item))
            },
            ActivityUpdate::TextSnapshot(text) => {
                presentation.approval =
                    matches!(presentation.kind, ActivityKind::ApprovalRequest { .. })
                        .then(|| ActivityApproval::from_snapshot(text))
                        .flatten();
                presentation.question =
                    matches!(presentation.kind, ActivityKind::UserInputRequest { .. })
                        .then(|| ActivityQuestion::from_snapshot(text))
                        .flatten();
                presentation.is_structured = presentation.kind == ActivityKind::ModelWork
                    && (ActivityNotice::from_snapshot(text).is_some()
                        || ActivitySummary::from_snapshot(text).is_some()
                        || ActivityReasoning::from_snapshot(text).is_some()
                        || ActivityPlan::from_snapshot(text).is_some()
                        || ActivityDocument::from_snapshot(text).is_some());
                let usage = (presentation.kind == ActivityKind::ModelWork)
                    .then(|| project_usage(activity, text))
                    .flatten();
                let text = match usage {
                    Some((text, summary)) => {
                        presentation.usage = summary;
                        text
                    },
                    None => {
                        presentation.usage = None;
                        project_snapshot(
                            presentation.kind,
                            presentation.question.as_ref().map_or_else(
                                || {
                                    presentation.approval.as_ref().map_or_else(
                                        || text.clone(),
                                        |approval| {
                                            request_text(
                                                &approval.plain_text,
                                                approval.choices.iter().map(|choice| {
                                                    (
                                                        choice.label.as_str(),
                                                        choice.description.as_str(),
                                                        choice.enabled,
                                                    )
                                                }),
                                            )
                                        },
                                    )
                                },
                                |question| {
                                    request_text(
                                        &question.plain_text,
                                        question.choices.iter().map(|choice| {
                                            (
                                                choice.label.as_str(),
                                                choice.description.as_str(),
                                                true,
                                            )
                                        }),
                                    )
                                },
                            ),
                        )
                    },
                };
                let visible = !text.is_empty();
                let changed = self
                    .transcript
                    .replace_text_changed(presentation.item, text)
                    .map_err(StateError::Transcript)?;
                presentation.has_payload = true;
                presentation.visible = visible;
                Ok(if changed {
                    ChatProjectionChange::VisibleItem(presentation.item)
                } else {
                    ChatProjectionChange::Unchanged
                })
            },
        }
    }

    fn start_activity(
        &mut self,
        activity: ActivityRef,
        kind: ActivityKind,
    ) -> Result<(TranscriptItemId, bool), StateError> {
        let id = self.next_transcript_id()?;
        if kind == ActivityKind::AgentMessage {
            self.transcript.start_markdown_assistant(id)
        } else if kind == ActivityKind::FileChange {
            self.transcript.start_file_change_message(id)
        } else if is_tool_activity(kind)
            || kind == ActivityKind::ModelWork
            || matches!(
                kind,
                ActivityKind::UserInputResponse { .. }
                    | ActivityKind::ApprovalResponse { .. }
                    | ActivityKind::UserInputRequest { .. }
                    | ActivityKind::ApprovalRequest { .. }
            )
        {
            self.transcript.start_typed_activity_message(id, kind)
        } else {
            self.transcript.start_assistant(id)
        }
        .map_err(StateError::Transcript)?;
        let label = activity_label(kind);
        if let Some(label) = label {
            self.transcript
                .append_text(id, label)
                .map_err(StateError::Transcript)?;
        }
        self.activities.insert(
            activity,
            ActivityPresentation {
                item: id,
                kind,
                has_payload: false,
                visible: label.is_some(),
                usage: None,
                is_structured: false,
                question: None,
                approval: None,
            },
        );
        Ok((id, label.is_some()))
    }

    fn finish_activity(
        &mut self,
        activity: ActivityRef,
        outcome: &ActivityOutcome,
    ) -> Result<(TranscriptItemId, bool), StateError> {
        let Some(presentation) = self.activities.remove(&activity) else {
            return Err(StateError::UnknownActivity(activity));
        };
        let id = presentation.item;
        let mut heading_changed = false;
        if matches!(
            presentation.kind,
            ActivityKind::ToolCall | ActivityKind::FileChange | ActivityKind::ModelWork
        ) {
            let item = self
                .transcript
                .items()
                .iter()
                .find(|item| item.id() == id)
                .expect("an active presentation owns a transcript item");
            let TranscriptBody::Message(message) = item.body();
            let text = message.text();
            let prefix = activity_label(presentation.kind).expect("these activities have headings");
            let replacement = text.strip_prefix(prefix).map(|payload| {
                let arguments_only = payload
                    .strip_prefix('\n')
                    .and_then(ToolOutput::from_snapshot)
                    .is_some_and(|output| {
                        // Search items describe an observed action without a result object.
                        output.tool != "webSearch"
                            && output.arguments.is_some()
                            && output.result.is_none()
                            && output.content_items.is_none()
                            && output.error.is_none()
                    });
                let label = match (presentation.kind, outcome) {
                    (ActivityKind::ToolCall, ActivityOutcome::Completed) if arguments_only => {
                        "Tool call prepared"
                    },
                    (ActivityKind::FileChange, ActivityOutcome::Completed) => "Changes completed",
                    (ActivityKind::FileChange, ActivityOutcome::Interrupted) => {
                        "Changes interrupted"
                    },
                    (ActivityKind::FileChange, ActivityOutcome::Failed(_)) => "Changes failed",
                    (ActivityKind::ModelWork, ActivityOutcome::Completed) => "Model work completed",
                    (ActivityKind::ModelWork, ActivityOutcome::Interrupted) => {
                        "Model work interrupted"
                    },
                    (ActivityKind::ModelWork, ActivityOutcome::Failed(_)) => "Model work failed",
                    (_, ActivityOutcome::Completed) => "Tool completed",
                    (_, ActivityOutcome::Interrupted) => "Tool interrupted",
                    (_, ActivityOutcome::Failed(_)) => "Tool failed",
                };
                format!("{label}{payload}")
            });
            if let Some(text) = replacement {
                heading_changed = self
                    .transcript
                    .replace_text_changed(id, text)
                    .map_err(StateError::Transcript)?;
            }
        }
        if is_tool_activity(presentation.kind)
            || presentation.is_structured
            || matches!(
                presentation.kind,
                ActivityKind::UserInputResponse { .. }
                    | ActivityKind::ApprovalResponse { .. }
                    | ActivityKind::UserInputRequest { .. }
                    | ActivityKind::ApprovalRequest { .. }
            )
        {
            let (status, footer) = match outcome {
                ActivityOutcome::Completed => (TranscriptActivityOutcome::Completed, None),
                ActivityOutcome::Interrupted => (
                    TranscriptActivityOutcome::Interrupted,
                    (!matches!(
                        presentation.kind,
                        ActivityKind::ToolCall | ActivityKind::FileChange
                    ))
                    .then(|| "\nInterrupted".to_owned()),
                ),
                ActivityOutcome::Failed(failure) => (
                    TranscriptActivityOutcome::Failed,
                    Some(format!("\nFailed: {}", failure.message())),
                ),
            };
            self.transcript
                .finish_activity_message(id, status, footer.as_deref())
                .map_err(StateError::Transcript)?;
            if presentation.kind == ActivityKind::FileChange && presentation.has_payload {
                self.retained_changes.insert(activity, id);
            }
            // Completing a request changes its status style, not its source context anchor.
            let context_changed = !matches!(
                (presentation.kind, outcome),
                (
                    ActivityKind::ApprovalRequest { .. } | ActivityKind::UserInputRequest { .. },
                    ActivityOutcome::Completed
                )
            );
            return Ok((id, context_changed));
        }
        let visible = match outcome {
            ActivityOutcome::Completed => heading_changed || !presentation.visible,
            ActivityOutcome::Interrupted if heading_changed => true,
            ActivityOutcome::Interrupted => self
                .transcript
                .append_outcome_footer(id, "\nInterrupted")
                .map(|()| true)
                .map_err(StateError::Transcript)?,
            ActivityOutcome::Failed(failure) => self
                .transcript
                .append_outcome_footer(id, &format!("\nFailed: {}", failure.message()))
                .map(|()| true)
                .map_err(StateError::Transcript)?,
        };
        if matches!(outcome, ActivityOutcome::Completed) && presentation.usage.is_some() {
            self.transcript.finish_usage_message(id)
        } else {
            self.transcript.finalize(id)
        }
        .map_err(StateError::Transcript)?;
        if matches!(outcome, ActivityOutcome::Completed)
            && let Some(usage) = presentation.usage
        {
            self.latest_usage = Some(usage);
        }
        Ok((id, visible))
    }

    pub(super) fn push_session_notice(
        &mut self,
        notice: ActivityNotice,
    ) -> Result<TranscriptItemId, StateError> {
        let Some(snapshot) = notice.to_snapshot() else {
            return self.push_notice(format!("{}\n{}", notice.title, notice.message));
        };
        let id = self.next_transcript_id()?;
        self.transcript
            .start_typed_activity_message(id, ActivityKind::ModelWork)
            .map_err(StateError::Transcript)?;
        self.transcript
            .append_text(id, &format!("Session notice\n{snapshot}"))
            .map_err(StateError::Transcript)?;
        self.transcript
            .finish_activity_message(id, TranscriptActivityOutcome::Completed, None)
            .map_err(StateError::Transcript)?;
        Ok(id)
    }

    pub(super) fn push_document(
        &mut self,
        document: TuiDocument,
    ) -> Result<TranscriptItemId, StateError> {
        let id = self.next_transcript_id()?;
        self.transcript
            .start_typed_activity_message(id, ActivityKind::ModelWork)
            .map_err(StateError::Transcript)?;
        self.transcript
            .append_text(id, &format!("Session document\n{}", document.snapshot()))
            .map_err(StateError::Transcript)?;
        self.transcript
            .finish_activity_message(id, TranscriptActivityOutcome::Completed, None)
            .map_err(StateError::Transcript)?;
        Ok(id)
    }

    pub(super) fn push_notice(&mut self, text: String) -> Result<TranscriptItemId, StateError> {
        let id = self.next_transcript_id()?;
        self.transcript
            .start_assistant(id)
            .map_err(StateError::Transcript)?;
        self.transcript
            .append_text(id, &text)
            .map_err(StateError::Transcript)?;
        self.transcript
            .finalize(id)
            .map_err(StateError::Transcript)?;
        Ok(id)
    }

    fn push_outcome_notice(
        &mut self,
        text: String,
        outcome: TranscriptActivityOutcome,
    ) -> Result<TranscriptItemId, StateError> {
        let id = self.next_transcript_id()?;
        self.transcript
            .start_activity_message(id)
            .map_err(StateError::Transcript)?;
        self.transcript
            .append_text(id, &text)
            .map_err(StateError::Transcript)?;
        self.transcript
            .finish_activity_message(id, outcome, None)
            .map_err(StateError::Transcript)?;
        Ok(id)
    }

    fn next_transcript_id(&mut self) -> Result<TranscriptItemId, StateError> {
        let id = TranscriptItemId::new(self.next_item_id);
        self.next_item_id = self
            .next_item_id
            .checked_add(1)
            .ok_or(StateError::ItemIdOverflow)?;
        Ok(id)
    }

    #[cfg(test)]
    pub(super) fn set_next_item_id(&mut self, value: u64) {
        self.next_item_id = value;
    }
}

fn project_usage(activity: ActivityRef, text: &str) -> Option<(String, Option<String>)> {
    if !text.trim_start().starts_with('{') {
        return None;
    }
    // Reuse the durable receipt validator on this single activity. No provider
    // wire fields or cumulative accounting rules belong in the Chat renderer.
    let records = [
        AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ModelWork,
        },
        AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(text.to_owned()),
        },
        AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        },
    ]
    .map(TranscriptRecord::EventCommitted);
    let projection = match SessionUsageProjection::from_records(&records) {
        Ok(projection) => projection,
        Err(error) => return Some((format!("Usage unavailable\n{}", error.detail()), None)),
    };
    let receipt = projection.receipts().first()?;
    let usage = receipt.usage();
    let input = usage_value(usage.input_tokens());
    let output = usage_value(usage.output_tokens());
    let mut detail = format!(
        "Token usage · latest observation\nInput: {input} · Output: {output}\nCache read: {} · Cache write: {}\nReasoning: {}",
        usage_value(usage.cache_read_input_tokens()),
        usage_value(usage.cache_write_input_tokens()),
        usage_value(usage.reasoning_tokens())
    );
    if let SessionUsageSource::Codex {
        model_context_window: Some(window),
        ..
    } = receipt.source()
    {
        detail.push_str(&format!("\nContext window: {window} tokens"));
    }
    Some((detail, Some(format!("Last: {input} in / {output} out"))))
}

fn usage_value(value: UsageValue) -> String {
    match value {
        UsageValue::Reported(tokens) => tokens.to_string(),
        UsageValue::Absent => "not reported".to_owned(),
        UsageValue::Unsupported => "unsupported".to_owned(),
    }
}

fn format_context_checkpoint(
    checkpoint: &ContextCheckpointObservation,
    policy: Option<&ContextPolicyChanged>,
) -> String {
    let retained_budget = policy
        .filter(|policy| policy.policy_revision() == checkpoint.policy_revision())
        .map_or_else(
            || "unknown".to_owned(),
            |policy| match (
                policy.retained_raw_percent(),
                policy.retained_raw_max_tokens(),
            ) {
                (Some(percent), Some(tokens)) => format!("{percent}% · max {tokens} tokens"),
                (Some(percent), None) => format!("{percent}%"),
                (None, Some(tokens)) => format!("max {tokens} tokens"),
                (None, None) => "none".to_owned(),
            },
        );
    let mut losses = vec![format!(
        "visible-prefix:{}",
        checkpoint.visible_prefix_loss_count()
    )];
    if checkpoint.provider_private_loss_count() > 0 {
        losses.push(format!(
            "provider-private:{}",
            checkpoint.provider_private_loss_count()
        ));
    }
    if checkpoint.image_input_loss_count() > 0 {
        losses.push(format!(
            "images summarized:{}",
            checkpoint.image_input_loss_count()
        ));
    }
    let counts = checkpoint.accounting().map_or_else(
        || {
            format!(
                "{} → {} tokens",
                checkpoint.input_tokens_before(),
                checkpoint.input_tokens_after()
            )
        },
        |(before, after)| {
            format!(
                "advisory estimate: {} + {} reserve → {} + {} reserve tokens",
                before.input_estimate(),
                before.reserve_tokens(),
                after.input_estimate(),
                after.reserve_tokens()
            )
        },
    );
    format!(
        "Context checkpoint committed\n{counts} (limit {})\ncontext epoch {} → {} · source {}..{}\nretained raw: {} group(s) (budget {retained_budget}) · artifact receipts: {} · losses: {}",
        checkpoint.input_token_limit(),
        checkpoint.previous_context_epoch(),
        checkpoint.successor_context_epoch(),
        checkpoint.source_anchor_sequence().get(),
        checkpoint.source_journal_boundary().get(),
        checkpoint.retained_group_count(),
        checkpoint.artifact_receipt_count(),
        losses.join(", "),
    )
}

// 선택지 설명은 plain_text에 포함된다고 가정하지 않고 원래 번호와 함께 보존한다.
fn request_text<'a>(
    plain_text: &str,
    choices: impl Iterator<Item = (&'a str, &'a str, bool)>,
) -> String {
    let mut text = plain_text.to_owned();
    for (index, (label, description, enabled)) in choices.enumerate() {
        if index == 0 {
            text.push_str("\n\nChoices:");
        }
        text.push_str(&format!(
            "\n{}. {}{}",
            index + 1,
            label,
            if enabled { "" } else { " (unavailable)" }
        ));
        if !description.is_empty() {
            text.push_str("\n   ");
            text.push_str(&description.replace('\n', "\n   "));
        }
    }
    text
}

fn project_snapshot(kind: ActivityKind, text: String) -> String {
    if kind == ActivityKind::ModelWork && text.starts_with("Plan\n") {
        return text;
    }
    if kind == ActivityKind::ModelWork
        && let Some(pressure) = project_context_pressure(&text)
    {
        return pressure;
    }
    let Some(label) = activity_label(kind) else {
        return text;
    };
    if text.is_empty() {
        label.to_owned()
    } else {
        format!("{label}\n{text}")
    }
}

fn project_context_pressure(text: &str) -> Option<String> {
    let observation = ContextPressureObservation::from_snapshot_json(text)?;
    let input_tokens = observation.input_tokens();
    let input_token_limit = observation.input_token_limit();
    let trigger_percent = observation.trigger_percent();
    let decision = match observation.decision() {
        ContextPressureDecision::Admit => "near compaction threshold",
        ContextPressureDecision::Compact => "compacting before the next request",
        ContextPressureDecision::Reject => "context limit reached",
    };
    let used_percent = if input_token_limit == 0 {
        100
    } else {
        u128::from(input_tokens)
            .saturating_mul(100)
            .checked_div(u128::from(input_token_limit))?
            .min(100) as u64
    };
    if let Some(accounting) = observation.accounting() {
        return Some(format!(
            "Context pressure\n{used_percent}% estimated ({} + {} reserve / {input_token_limit} tokens) · advisory estimate · {decision} · compacts at {trigger_percent}%",
            accounting.input_estimate(),
            accounting.reserve_tokens(),
        ));
    }
    Some(format!(
        "Context pressure\n{used_percent}% used ({input_tokens} / {input_token_limit} tokens) · {decision} · compacts at {trigger_percent}%"
    ))
}

const fn is_tool_activity(kind: ActivityKind) -> bool {
    matches!(
        kind,
        ActivityKind::ToolCall | ActivityKind::ToolResult | ActivityKind::FileChange
    )
}

const fn activity_label(kind: ActivityKind) -> Option<&'static str> {
    match kind {
        ActivityKind::ModelWork => Some("Thinking…"),
        ActivityKind::AgentMessage => None,
        ActivityKind::ToolCall => Some("Running tool…"),
        ActivityKind::ToolResult => Some("Tool result"),
        ActivityKind::FileChange => Some("File change observed"),
        ActivityKind::ApprovalRequest { .. } => Some("Approval required"),
        ActivityKind::ApprovalResponse { .. } => Some("Approval response sent"),
        ActivityKind::UserInputRequest { .. } => Some("Answer requested"),
        ActivityKind::UserInputResponse { .. } => Some("Answer recorded"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 상속 기록은 완료된 표현만 복사하고 원본 활동 식별자와 실행 상태를 가져오지 않습니다.
    #[test]
    fn inherited_records_copy_final_typed_items_without_execution_state() {
        use std::num::NonZeroU64;

        use yo_core::{ActivityId, SessionId, TurnId, TurnRef, UserInput};

        use crate::{
            appearance::{AppearanceCandidate, AppearanceState, GlyphProfile},
            transcript::prepare,
        };

        let session: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
        let turn = TurnRef::new(session, TurnId::new(NonZeroU64::new(1).unwrap()));
        let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
        let records = vec![
            TranscriptRecord::CommandCommitted(AgentCommand::StartTurn { turn, input: UserInput::new("inherited question") }),
            TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted { activity, kind: ActivityKind::AgentMessage }),
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { activity, update: ActivityUpdate::TextSnapshot("```rust\nfn inherited() {}\n```\n\n| Name | Value |\n| --- | --- |\n| retained | exact |".to_owned()) }),
            TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished { activity, outcome: ActivityOutcome::Completed }),
        ];
        let mut chat = ChatProjection::new();
        chat.push_notice("child notice".to_owned()).unwrap();
        chat.observe_inherited_records(&records).unwrap();
        chat.observe_inherited_records(&records).unwrap();
        assert!(chat.activities.is_empty());
        assert!(chat.retained_changes.is_empty());
        assert!(chat.context_policy.is_none());
        assert!(chat.latest_usage().is_none());
        assert!(chat.approval(activity).is_none());
        assert!(chat.question(activity).is_none());
        assert_eq!(chat.transcript.items().len(), 5);
        for (index, item) in chat.transcript.items().iter().enumerate() {
            assert_eq!(item.id().get(), index as u64 + 1);
            assert_eq!(item.phase(), TranscriptPhase::Final);
        }
        let appearance =
            AppearanceState::new(AppearanceCandidate::for_profile(GlyphProfile::Rich)).unwrap();
        let pin = appearance.pin();
        for width in [20, 40] {
            prepare(chat.transcript(), width, pin.snapshot().transcript_config()).unwrap();
        }
        // 미완료 원문을 조용히 생략하지 않고 상속 projection 실패를 반환합니다.
        let mut incomplete = records.clone();
        incomplete.pop();
        assert!(chat.observe_inherited_records(&incomplete).is_err());
        assert!(chat.publication_candidate().is_some());
    }

    // context pressure receipt는 durable typed JSON을 유지하되 Chat에는 model reasoning처럼
    // 원문을 노출하지 않고 사람이 읽을 수 있는 pressure 상태로 투영합니다.
    #[test]
    fn projects_context_pressure_without_exposing_raw_telemetry() {
        let projected = project_snapshot(
            ActivityKind::ModelWork,
            ContextPressureObservation::new(86, 100, 85, 90, ContextPressureDecision::Admit)
                .unwrap()
                .to_snapshot_json(),
        );

        assert_eq!(
            projected,
            "Context pressure\n86% used (86 / 100 tokens) · near compaction threshold · compacts at 90%"
        );
        assert!(!projected.contains("yo.context-pressure"));
        assert!(!projected.contains("Thinking"));
    }

    // 이미지 계획치는 실제 사용량으로 표시하지 않고 추정과 요청당 reserve를 함께 보여 준다.
    #[test]
    fn image_pressure_labels_advisory_estimate_and_reserve() {
        let projected = project_snapshot(ActivityKind::ModelWork,
            r#"{"schema":"yo.context-pressure/v2alpha1","accounting":{"quality":"advisory_estimate","policy":"kimi-code-image-advisory/v1","input_estimate":7600,"reserve_tokens":1024},"input_token_limit":10000,"warning_percent":85,"trigger_percent":90,"decision":"admit"}"#.to_owned());
        assert!(projected.contains("86% estimated"));
        assert!(projected.contains("7600 + 1024 reserve"));
        assert!(projected.contains("advisory estimate"));
        assert!(!projected.contains("% used"));
        assert!(!projected.contains("yo.context-pressure"));
    }
}
