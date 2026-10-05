use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    mem,
};

use sha2::Digest as _;

use super::super::{
    JournalEntry, JournalSequence, SemanticRecord, SessionJournal,
    codec::{
        BindingCloseReason, ContextArtifactReceipt, ContextCheckpoint, ContextImageLoss,
        ContextImageSource, ContextLoss, ContextPolicyChanged, ContextRetainedGroup,
        ContextStrategy, ContextSummaryUsage, ForkSeed, TransitionMode, validate_image_losses,
    },
    read_state,
};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRef, AgentCommand, AgentEvent,
    ContextCheckpointProposal, ModelReplay, ModelReplayItem, ModelReplayRole, ReplayProfile,
    TurnOutcome, TurnRef,
    backend::{provider_private_schema, validate_provider_private_replay_sequence},
};

#[derive(Default)]
struct ActiveSourceEvidence {
    accepted_request: Option<JournalSequence>,
    last_activity_boundary: Option<JournalSequence>,
    last_activity_kind: Option<ActivityKind>,
    last_agent_message_boundary: Option<JournalSequence>,
    last_tool_call_boundary: Option<JournalSequence>,
    last_question_response_boundary: Option<JournalSequence>,
    question_responses: Vec<(crate::ActivityQuestion, crate::ActivityResponse)>,
    last_interrupted_activity_boundary: Option<JournalSequence>,
    last_activity_completed: bool,
    has_open_activity: bool,
}

fn active_source_evidence(
    entries: &[JournalEntry],
    turn: TurnRef,
    first: JournalSequence,
    last: JournalSequence,
) -> ActiveSourceEvidence {
    let mut evidence = ActiveSourceEvidence::default();
    let mut started_activities = BTreeMap::<ActivityRef, ActivityKind>::new();
    let mut ordinary_questions = BTreeMap::new();
    let mut answered_questions = BTreeMap::new();
    let mut completed_questions = BTreeMap::new();
    for entry in entries.iter().filter(|entry| entry.sequence() <= last) {
        match entry.record() {
            SemanticRecord::BackendRequestAccepted(request)
                if request.turn_id() == turn.turn_id() && entry.sequence() >= first =>
            {
                evidence.accepted_request = Some(entry.sequence());
            },
            SemanticRecord::EventCommitted(AgentEvent::ActivityStarted { activity, kind })
                if activity.turn() == turn =>
            {
                started_activities.insert(*activity, *kind);
            },
            SemanticRecord::EventCommitted(AgentEvent::ActivityUpdated {
                activity,
                update: crate::ActivityUpdate::TextSnapshot(snapshot),
            }) if activity.turn() == turn => {
                if let Some(ActivityKind::UserInputRequest { request_id }) =
                    started_activities.get(activity)
                {
                    let request = crate::ActivityRequestRef::new(*activity, *request_id);
                    if let Some(question) = crate::ActivityQuestion::from_snapshot(snapshot)
                        .filter(|question| question.allow_unanswered && !question.is_secret)
                    {
                        ordinary_questions.insert(request, question);
                    } else {
                        ordinary_questions.remove(&request);
                    }
                }
            },
            SemanticRecord::CommandCommitted(command) if entry.sequence() >= first => {
                if let AgentCommand::RespondToActivity {
                    request,
                    response:
                        response @ (crate::ActivityResponse::UserInput(_)
                        | crate::ActivityResponse::QuestionAnswer { .. }
                        | crate::ActivityResponse::QuestionUnanswered),
                } = command.command()
                    && let Some(question) = ordinary_questions.get(request)
                {
                    answered_questions.insert(*request, (question.clone(), response.clone()));
                }
            },
            SemanticRecord::EventCommitted(AgentEvent::ActivityFinished { activity, outcome })
                if activity.turn() == turn =>
            {
                if let Some(kind) = started_activities.remove(activity)
                    && entry.sequence() >= first
                {
                    if matches!(outcome, ActivityOutcome::Interrupted) {
                        evidence.last_interrupted_activity_boundary = Some(entry.sequence());
                    } else {
                        evidence.last_activity_boundary = Some(entry.sequence());
                        evidence.last_activity_kind = Some(kind);
                        evidence.last_activity_completed =
                            matches!(outcome, ActivityOutcome::Completed);
                        if matches!(outcome, ActivityOutcome::Completed) {
                            match kind {
                                ActivityKind::AgentMessage => {
                                    evidence.last_agent_message_boundary = Some(entry.sequence());
                                },
                                ActivityKind::ToolCall => {
                                    evidence.last_tool_call_boundary = Some(entry.sequence());
                                },
                                ActivityKind::UserInputRequest { request_id }
                                    if answered_questions.contains_key(
                                        &crate::ActivityRequestRef::new(*activity, request_id),
                                    ) =>
                                {
                                    let response = answered_questions
                                        .remove(&crate::ActivityRequestRef::new(
                                            *activity, request_id,
                                        ))
                                        .expect("matched ordinary question response");
                                    completed_questions.insert(request_id, response);
                                },
                                ActivityKind::UserInputResponse { request_id }
                                    if completed_questions.contains_key(&request_id) =>
                                {
                                    evidence.last_question_response_boundary =
                                        Some(entry.sequence());
                                    evidence.question_responses.push(
                                        completed_questions
                                            .remove(&request_id)
                                            .expect("matched completed question"),
                                    );
                                },
                                _ => {},
                            }
                        }
                    }
                }
            },
            _ => {},
        }
    }
    evidence.has_open_activity = !started_activities.is_empty();
    evidence
}

fn active_suffix_matches(
    entries: &[JournalEntry],
    turn: TurnRef,
    first: JournalSequence,
    last: JournalSequence,
    items: &[ModelReplayItem],
) -> bool {
    let Some(expected_inputs) = committed_user_inputs(entries, turn, first, last) else {
        return false;
    };
    if replay_user_inputs(items) != expected_inputs {
        return false;
    }
    if items.iter().any(|item| {
        !matches!(
            item,
            ModelReplayItem::MultimodalUser { .. }
                | ModelReplayItem::Message {
                    role: ModelReplayRole::User | ModelReplayRole::Assistant,
                    ..
                }
                | ModelReplayItem::FunctionCall { .. }
                | ModelReplayItem::FunctionCallOutput { .. }
                | ModelReplayItem::ProviderPrivateAssistant { .. }
        )
    }) {
        return false;
    }
    let evidence = active_source_evidence(entries, turn, first, last);
    if evidence.has_open_activity || !question_results_match(items, &evidence.question_responses) {
        return false;
    }
    let last_input_sequence = entries
        .iter()
        .filter(|entry| first <= entry.sequence() && entry.sequence() <= last)
        .filter_map(|entry| match entry.record() {
            SemanticRecord::CommandCommitted(committed)
                if matches!(
                    committed.command(),
                    AgentCommand::StartTurn { turn: candidate, .. }
                        | AgentCommand::SteerTurn { turn: candidate, .. }
                        if *candidate == turn
                ) =>
            {
                Some(entry.sequence())
            },
            _ => None,
        })
        .next_back();
    let checkpoint_root = entries
        .iter()
        .filter(|entry| entry.sequence() <= last)
        .filter_map(|entry| {
            matches!(entry.record(), SemanticRecord::ContextCheckpoint(_))
                .then_some(entry.sequence())
        })
        .next_back()
        == Some(first);
    if last_input_sequence.is_none() && !checkpoint_root {
        return false;
    }
    let has_assistant_message = items.iter().any(|item| {
        matches!(
            item,
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                ..
            }
        )
    });
    let has_tool_call = items
        .iter()
        .any(|item| matches!(item, ModelReplayItem::FunctionCall { .. }));
    let has_tool_output = items
        .iter()
        .any(|item| matches!(item, ModelReplayItem::FunctionCallOutput { .. }));
    let has_tool_items = has_tool_call || has_tool_output;
    let only_active_inputs = items.iter().all(|item| {
        matches!(
            item,
            ModelReplayItem::MultimodalUser { .. }
                | ModelReplayItem::Message {
                    role: ModelReplayRole::User,
                    ..
                }
        )
    });

    let Some(request) = evidence.accepted_request else {
        return only_active_inputs && last_input_sequence == Some(last);
    };
    if evidence
        .last_interrupted_activity_boundary
        .is_some_and(|boundary| request < boundary)
    {
        return false;
    }
    let Some(last_activity) = evidence.last_activity_boundary else {
        return false;
    };
    let response_precedes_tail = evidence
        .last_agent_message_boundary
        .into_iter()
        .chain(evidence.last_tool_call_boundary)
        .chain(evidence.last_question_response_boundary)
        .any(|response| request < response && response < last_activity);
    let activity_end_is_closed_response = match evidence.last_activity_kind {
        Some(ActivityKind::AgentMessage) => {
            evidence.last_agent_message_boundary == Some(last_activity)
        },
        Some(ActivityKind::ToolCall) => evidence.last_tool_call_boundary == Some(last_activity),
        Some(ActivityKind::UserInputResponse { .. })
            if evidence.last_question_response_boundary == Some(last_activity) =>
        {
            true
        },
        Some(ActivityKind::ToolResult) => evidence
            .last_tool_call_boundary
            .is_some_and(|tool_call| tool_call < last_activity),
        Some(_) => evidence.last_activity_completed && response_precedes_tail,
        None => false,
    };
    let assistant_completed = evidence
        .last_agent_message_boundary
        .is_some_and(|boundary| request < boundary);
    if has_assistant_message && assistant_completed && activity_end_is_closed_response {
        return last == last_input_sequence.map_or(last_activity, |input| input.max(last_activity))
            && (!has_tool_items || (has_tool_call && has_tool_output));
    }
    let tool_call_completed = evidence
        .last_tool_call_boundary
        .is_some_and(|boundary| request < boundary);
    let question_completed = evidence
        .last_question_response_boundary
        .is_some_and(|boundary| request < boundary)
        && !evidence.question_responses.is_empty();
    has_tool_call
        && has_tool_output
        && (tool_call_completed || question_completed)
        && activity_end_is_closed_response
        && last == last_input_sequence.map_or(last_activity, |input| input.max(last_activity))
}

// ordinary 결과를 생성하지 않고, durable 명령과 typed 선택지가 그 결과의 유일한 근거인지
// 확인합니다.
fn question_results_match(
    items: &[ModelReplayItem],
    responses: &[(crate::ActivityQuestion, crate::ActivityResponse)],
) -> bool {
    let calls = items
        .iter()
        .filter_map(|item| match item {
            ModelReplayItem::FunctionCall { call_id, name, .. } if name == "ask_user" => {
                Some(call_id)
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    if calls.len() != responses.len() {
        return false;
    }
    let outputs = items
        .iter()
        .filter_map(|item| match item {
            ModelReplayItem::FunctionCallOutput { call_id, output } => Some((call_id, output)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    calls
        .iter()
        .zip(responses)
        .all(|(call_id, (question, response))| {
            let Some(output) = outputs.get(call_id) else {
                return false;
            };
            let Ok(serde_json::Value::Object(value)) = serde_json::from_str(output) else {
                return false;
            };
            let field = |name: &str| value.get(name).and_then(serde_json::Value::as_str);
            if field("schema") != Some("yo.ask-user-result/v1") {
                return false;
            }
            match response {
                crate::ActivityResponse::QuestionUnanswered => {
                    value.len() == 2 && field("status") == Some("unanswered")
                },
                crate::ActivityResponse::UserInput(input) => {
                    value.len() == 4
                        && field("status") == Some("answered")
                        && field("kind") == Some("text")
                        && field("text") == Some(input.as_str())
                },
                crate::ActivityResponse::QuestionAnswer { choice, notes } => {
                    let label = choice
                        .checked_sub(1)
                        .and_then(|ordinal| question.choices.get(ordinal as usize))
                        .map(|choice| choice.label.as_str());
                    value.len() == 6
                        && field("status") == Some("answered")
                        && field("kind") == Some("choice")
                        && value.get("choice").and_then(serde_json::Value::as_u64)
                            == Some(u64::from(*choice))
                        && label.is_some()
                        && field("label") == label
                        && field("notes") == Some(notes.as_str())
                },
                _ => false,
            }
        })
}

#[derive(Clone)]
pub(super) struct ContextSourceGroup {
    pub(super) first_sequence: JournalSequence,
    pub(super) last_sequence: JournalSequence,
    pub(super) replay_sequence: JournalSequence,
    pub(super) items: Vec<ModelReplayItem>,
    pub(super) fork_import: Option<(JournalSequence, usize)>,
    pub(super) private_epochs: Vec<u64>,
    pub(super) image_losses: Vec<ContextImageLoss>,
}

#[derive(Clone)]
pub(crate) struct ContextActiveSource {
    turn: TurnRef,
    first_sequence: JournalSequence,
    last_sequence: JournalSequence,
    items: Vec<ModelReplayItem>,
}

impl ContextActiveSource {
    pub(crate) fn new(
        turn: TurnRef,
        first_sequence: JournalSequence,
        last_sequence: JournalSequence,
        items: Vec<ModelReplayItem>,
    ) -> Self {
        Self {
            turn,
            first_sequence,
            last_sequence,
            items,
        }
    }

    pub(crate) fn try_advance(
        &mut self,
        turn: TurnRef,
        last_sequence: JournalSequence,
        items: Vec<ModelReplayItem>,
    ) -> bool {
        if self.turn != turn
            || last_sequence <= self.last_sequence
            || items.len() <= self.items.len()
            || !items.starts_with(&self.items)
        {
            return false;
        }
        self.last_sequence = last_sequence;
        self.items = items;
        true
    }
}

impl SessionJournal {
    pub(crate) fn advance_active_context_source(
        &self,
        source: &mut ContextActiveSource,
        turn: TurnRef,
        last_sequence: JournalSequence,
        items: Vec<ModelReplayItem>,
    ) -> bool {
        let mut candidate = source.clone();
        if !candidate.try_advance(turn, last_sequence, items) {
            return false;
        }
        let entries = self.semantic_entries();
        let Some(root) = entries
            .iter()
            .find(|entry| entry.sequence() == candidate.first_sequence)
        else {
            return false;
        };
        match root.record() {
            SemanticRecord::CommandCommitted(committed) => match committed.command() {
                AgentCommand::StartTurn {
                    turn: candidate_turn,
                    input,
                } if *candidate_turn == turn
                    && candidate.items.first() == Some(&input.model_replay_item()) => {},
                _ => return false,
            },
            SemanticRecord::ContextCheckpoint(_) => {},
            _ => return false,
        }
        let Some(last) = entries
            .iter()
            .find(|entry| entry.sequence() == candidate.last_sequence)
        else {
            return false;
        };
        let closed_boundary = match last.record() {
            SemanticRecord::EventCommitted(AgentEvent::ActivityFinished { activity, .. }) => {
                activity.turn() == turn
            },
            SemanticRecord::CommandCommitted(committed) => matches!(
                committed.command(),
                AgentCommand::SteerTurn {
                    turn: candidate_turn,
                    ..
                } if *candidate_turn == turn
            ),
            _ => false,
        };
        if !closed_boundary
            || !active_suffix_matches(
                &entries,
                turn,
                candidate.first_sequence,
                candidate.last_sequence,
                &candidate.items,
            )
        {
            return false;
        }
        *source = candidate;
        true
    }

    pub(crate) fn append_context_policy(&mut self, policy: ContextPolicyChanged) -> bool {
        let records = vec![SemanticRecord::ContextPolicyChanged(policy)];
        if self.durable.is_none() {
            self.append_records(records);
            true
        } else {
            self.append_records_transactionally(records)
        }
    }

    pub(crate) fn commit_context_checkpoint(
        &mut self,
        proposal: &ContextCheckpointProposal,
        policy: &ContextPolicyChanged,
        epoch: u64,
        previous_context_epoch: u64,
        source_anchor_sequence: JournalSequence,
        active_source: Option<&ContextActiveSource>,
    ) -> Option<(JournalSequence, ModelReplay)> {
        if !policy.enabled()
            || policy.strategy() != ContextStrategy::PortableSummaryV1Alpha1
            || policy.policy_revision() != proposal.policy_revision()
            || u128::from(proposal.input_tokens_after()) * 100
                >= u128::from(proposal.input_token_limit()) * u128::from(policy.trigger_percent())
        {
            return None;
        }
        let entries = self.semantic_entries();
        let groups = context_source_groups(&entries, epoch, previous_context_epoch)?;
        let expected_groups = proposal
            .summarized_groups()
            .iter()
            .chain(proposal.retained_groups())
            .collect::<Vec<_>>();
        if expected_groups.len() != groups.len()
            || expected_groups
                .iter()
                .zip(&groups)
                .any(|(expected, source)| expected.as_slice() != source.items)
        {
            return None;
        }
        let summarized_count = proposal.summarized_groups().len();
        if summarized_count == 0 || summarized_count > groups.len() {
            return None;
        }
        let anchor = entries.iter().find_map(|entry| {
            (entry.sequence() == source_anchor_sequence).then(|| match entry.record() {
                SemanticRecord::ContinuationAnchor(anchor) => Some(anchor),
                _ => None,
            })?
        })?;
        if anchor.epoch() != epoch || anchor.context_epoch() != Some(previous_context_epoch) {
            return None;
        }
        let mut retained = groups[summarized_count..]
            .iter()
            .map(|group| {
                match group.fork_import {
                    Some((seed, index)) => ContextRetainedGroup::try_imported(
                        seed,
                        index,
                        group.items.clone(),
                        group.private_epochs.clone(),
                    ),
                    None => ContextRetainedGroup::try_new(
                        group.first_sequence,
                        group.last_sequence,
                        group.items.clone(),
                    ),
                }
                .ok()
            })
            .collect::<Option<Vec<_>>>()?;
        let source_journal_boundary = if let Some(turn) = proposal.turn() {
            let source = active_source?;
            if source.turn != turn || source.last_sequence < source.first_sequence {
                return None;
            }
            let root = entries
                .iter()
                .find(|entry| entry.sequence() == source.first_sequence)?;
            let start_input = match root.record() {
                SemanticRecord::CommandCommitted(committed) => match committed.command() {
                    AgentCommand::StartTurn {
                        turn: candidate,
                        input,
                    } if *candidate == turn => Some(input.model_replay_item()),
                    _ => return None,
                },
                SemanticRecord::ContextCheckpoint(checkpoint)
                    if checkpoint.epoch() == epoch
                        && checkpoint.successor_context_epoch() == previous_context_epoch =>
                {
                    None
                },
                _ => return None,
            };
            if start_input
                .as_ref()
                .is_some_and(|input| source.items.first() != Some(input))
                || replay_user_inputs(&source.items)
                    != committed_user_inputs(
                        &entries,
                        turn,
                        source.first_sequence,
                        source.last_sequence,
                    )?
                || entries
                    .iter()
                    .find(|entry| entry.sequence() == source.last_sequence)
                    .is_none_or(|entry| {
                        source.last_sequence != source.first_sequence
                            && !matches!(
                                entry.record(),
                                SemanticRecord::EventCommitted(AgentEvent::ActivityFinished {
                                    activity,
                                    ..
                                }) if activity.turn() == turn
                            )
                            && !matches!(
                                entry.record(),
                                SemanticRecord::CommandCommitted(committed)
                                    if matches!(
                                        committed.command(),
                                        AgentCommand::SteerTurn {
                                            turn: candidate,
                                            ..
                                        } if *candidate == turn
                                    )
                            )
                    })
            {
                return None;
            }
            let mut trailing_inputs = Vec::new();
            let mut trailing_boundary = None;
            for entry in entries
                .iter()
                .filter(|entry| entry.sequence() > source.last_sequence)
            {
                let SemanticRecord::CommandCommitted(committed) = entry.record() else {
                    continue;
                };
                match committed.command() {
                    AgentCommand::SteerTurn {
                        turn: candidate,
                        input,
                    } if *candidate == turn => {
                        trailing_inputs.push(input.model_replay_item());
                        trailing_boundary = Some(entry.sequence());
                    },
                    AgentCommand::StartTurn { .. } | AgentCommand::SteerTurn { .. } => {
                        return None;
                    },
                    AgentCommand::CreateSession { .. }
                    | AgentCommand::RespondToActivity { .. }
                    | AgentCommand::InterruptTurn { .. }
                    | AgentCommand::CompactContext { .. } => return None,
                }
            }
            if !proposal.active_group().starts_with(source.items.as_slice())
                || proposal.active_group()[source.items.len()..] != trailing_inputs
                || replay_user_inputs(proposal.active_group())
                    != committed_user_inputs(
                        &entries,
                        turn,
                        source.first_sequence,
                        trailing_boundary.unwrap_or(source.last_sequence),
                    )?
            {
                return None;
            }
            let boundary = trailing_boundary.unwrap_or(source.last_sequence);
            if !active_suffix_matches(
                &entries,
                turn,
                source.first_sequence,
                boundary,
                proposal.active_group(),
            ) || entries.iter().any(|entry| {
                entry.sequence() > boundary
                    && matches!(
                        entry.record(),
                        SemanticRecord::BackendRequestAccepted(request)
                            if request.turn_id() == turn.turn_id()
                    )
            }) {
                return None;
            }
            retained.push(
                ContextRetainedGroup::try_new(
                    source.first_sequence,
                    boundary,
                    proposal.active_group().to_vec(),
                )
                .ok()?,
            );
            boundary
        } else {
            if active_source.is_some() || !proposal.active_group().is_empty() {
                return None;
            }
            anchor.journal_boundary()
        };
        let summarized = &groups[..summarized_count];
        let mut receipts = Vec::new();
        let mut receipt_identities = BTreeSet::new();
        let mut losses = vec![
            ContextLoss::visible_prefix_summarized(
                summarized.iter().map(|group| group.first_sequence).min()?,
                summarized.iter().map(|group| group.last_sequence).max()?,
            )
            .ok()?,
        ];
        for group in summarized {
            for item in &group.items {
                match item {
                    ModelReplayItem::FunctionCallOutput { output, .. } if !output.is_empty() => {
                        let digest = sha2::Sha256::digest(output.as_bytes());
                        let mut content_hash = String::from("sha256:");
                        for byte in digest {
                            write!(&mut content_hash, "{byte:02x}")
                                .expect("writing a digest into a String cannot fail");
                        }
                        let byte_count = u64::try_from(output.len()).ok()?;
                        if receipt_identities.insert((
                            group.replay_sequence,
                            content_hash.clone(),
                            byte_count,
                        )) {
                            receipts.push(
                                ContextArtifactReceipt::try_new(
                                    content_hash,
                                    byte_count,
                                    "text/plain",
                                    previous_context_epoch,
                                    group.replay_sequence,
                                )
                                .ok()?,
                            );
                        }
                    },
                    ModelReplayItem::ProviderPrivateAssistant { envelope } => {
                        losses.push(
                            ContextLoss::provider_private_dropped(
                                envelope.schema(),
                                u64::try_from(envelope.payload().len()).ok()?,
                                group.replay_sequence,
                            )
                            .ok()?,
                        );
                    },
                    _ => {},
                }
            }
        }
        let mut image_losses = Vec::new();
        for loss in summarized.iter().flat_map(|group| &group.image_losses) {
            if image_losses.len() == 64 {
                return None;
            }
            image_losses.push(loss.clone());
            validate_image_losses(&image_losses).ok()?;
        }
        losses.extend(
            image_losses
                .into_iter()
                .map(ContextLoss::ImageInputSummarized),
        );
        let first_retained_sequence = retained.first().map(ContextRetainedGroup::first_sequence);
        let mut checkpoint = ContextCheckpoint::try_new(
            epoch,
            previous_context_epoch,
            previous_context_epoch.checked_add(1)?,
            source_anchor_sequence,
            source_journal_boundary,
            policy.policy_revision(),
            policy.strategy(),
            proposal.input_token_limit(),
            proposal.input_tokens_before(),
            proposal.input_tokens_after(),
            proposal.replay_contract().clone(),
            proposal.portable_body(),
            retained,
            first_retained_sequence,
            receipts,
            losses,
            ContextSummaryUsage::try_new(proposal.summary_usage().clone()).ok()?,
        )
        .ok()?;
        if let Some((before, after)) = proposal.accounting() {
            checkpoint = checkpoint
                .with_accounting(before.clone(), after.clone())
                .ok()?;
        }
        checkpoint.validate_profile().ok()?;
        let binding = entries.iter().find_map(|entry| match entry.record() {
            SemanticRecord::BackendBindingOpened(binding) if binding.epoch() == epoch => {
                Some(binding)
            },
            _ => None,
        })?;
        let retained_items = checkpoint
            .retained_groups()
            .iter()
            .flat_map(|group| group.items())
            .cloned()
            .collect::<Vec<_>>();
        let has_provider_private = retained_items
            .iter()
            .any(|item| matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. }));
        match binding.continuation_strategy() {
            crate::ContinuationStrategy::ExactReplay {
                replay_profile: ReplayProfile::SemanticOnly,
                ..
            } if has_provider_private => return None,
            crate::ContinuationStrategy::ExactReplay {
                replay_profile: ReplayProfile::ProviderPrivateLocalPlaintext,
                ..
            } => {
                if retained_items.iter().any(|item| {
                    matches!(
                        item,
                        ModelReplayItem::Message {
                            role: ModelReplayRole::Assistant,
                            ..
                        }
                    )
                }) {
                    validate_provider_private_replay_sequence(
                        &retained_items,
                        provider_private_schema(ReplayProfile::ProviderPrivateLocalPlaintext)?,
                    )
                    .ok()?;
                } else if has_provider_private {
                    return None;
                }
            },
            _ => {},
        }
        checkpoint
            .validate_binding_accounting(binding.binding_identity().value())
            .ok()?;
        let replay = checkpoint.replay_root().ok()?;
        let sequence = read_state(&self.state).next_sequence();
        let records = vec![SemanticRecord::ContextCheckpoint(checkpoint)];
        let committed = if self.durable.is_none() {
            self.append_records(records);
            true
        } else {
            self.append_records_transactionally(records)
        };
        committed.then_some((sequence, replay))
    }
}

fn committed_user_inputs(
    entries: &[JournalEntry],
    turn: TurnRef,
    first: JournalSequence,
    last: JournalSequence,
) -> Option<Vec<ModelReplayItem>> {
    let mut inputs = Vec::new();
    for entry in entries
        .iter()
        .filter(|entry| first <= entry.sequence() && entry.sequence() <= last)
    {
        let SemanticRecord::CommandCommitted(committed) = entry.record() else {
            continue;
        };
        match committed.command() {
            AgentCommand::StartTurn {
                turn: candidate,
                input,
            }
            | AgentCommand::SteerTurn {
                turn: candidate,
                input,
            } => {
                if *candidate != turn || committed.submission_id().is_none() {
                    return None;
                }
                inputs.push(input.model_replay_item());
            },
            _ => {},
        }
    }
    Some(inputs)
}

fn replay_user_inputs(items: &[ModelReplayItem]) -> Vec<ModelReplayItem> {
    items
        .iter()
        .filter(|item| {
            matches!(
                item,
                ModelReplayItem::MultimodalUser { .. }
                    | ModelReplayItem::Message {
                        role: ModelReplayRole::User,
                        ..
                    }
            )
        })
        .cloned()
        .collect()
}

pub(super) fn context_source_groups(
    entries: &[JournalEntry],
    epoch: u64,
    context_epoch: u64,
) -> Option<Vec<ContextSourceGroup>> {
    let mut groups = Vec::new();
    let mut registered = None;
    let mut owner_epoch = None;
    let mut accepted_since_seed = false;
    let mut current_context = None;
    let mut closed_owner = None;
    let mut source_anchor = None;
    let mut source_checkpoint = None;
    for (index, entry) in entries.iter().enumerate() {
        match entry.record() {
            SemanticRecord::InitialForkSeed(seed) => {
                if registered.is_some() {
                    return None;
                }
                registered = Some((entry.sequence(), seed));
            },
            SemanticRecord::BackendRequestAccepted(_) if registered.is_some() => {
                accepted_since_seed = true;
                source_anchor = None;
                source_checkpoint = None;
            },
            SemanticRecord::BackendBindingClosed(binding) if registered.is_some() => {
                if owner_epoch != Some(binding.epoch()) {
                    return None;
                }
                closed_owner = Some((binding.epoch(), binding.reason()));
            },
            SemanticRecord::BackendBindingOpened(binding) if registered.is_some() => {
                let (seed_sequence, seed) = registered?;
                if matches!(seed.seed(), ForkSeed::Empty) {
                    // Empty forks carry no imported baseline; ordinary recovery owns bindings.
                    registered = None;
                    owner_epoch = Some(binding.epoch());
                    current_context = Some(1);
                    continue;
                }
                let ForkSeed::ExactReplay(replay) = seed.seed() else {
                    return None;
                };
                let source = seed.source().point()?.binding();
                if (owner_epoch.is_none()
                    || groups
                        .iter()
                        .any(|group: &ContextSourceGroup| group.fork_import.is_some()))
                    && (binding.backend_kind() != source.backend_kind()
                        || binding.binding_identity().schema()
                            != source.binding_identity().schema()
                        || binding.binding_identity().value() != source.binding_identity().value()
                        || binding.model_identity().schema() != source.model_identity().schema()
                        || binding.model_identity().value() != source.model_identity().value()
                        || binding.continuation_strategy() != source.continuation_strategy())
                {
                    return None;
                }
                if owner_epoch.is_none() {
                    if binding.epoch() != 1
                        || binding.transition().mode() != TransitionMode::InitialFork
                        || binding.transition().fork_seed_sequence() != Some(seed_sequence)
                    {
                        return None;
                    }
                    groups = replay
                        .groups()
                        .iter()
                        .enumerate()
                        .map(|(group_index, group)| {
                            let range = group.first_item()..group.end_item();
                            Some(ContextSourceGroup {
                                first_sequence: seed_sequence,
                                last_sequence: seed_sequence,
                                replay_sequence: seed_sequence,
                                image_losses: ContextImageLoss::for_items(
                                    &replay.items()[range.clone()],
                                    1,
                                    |item_index, part_index| ContextImageSource::InitialForkSeed {
                                        sequence: seed_sequence.get(),
                                        group_index: group_index as u32,
                                        item_index,
                                        part_index,
                                    },
                                )
                                .ok()?,
                                items: replay.items()[range.clone()].to_vec(),
                                fork_import: Some((seed_sequence, group_index)),
                                private_epochs: replay.items()[range.clone()]
                                    .iter()
                                    .zip(&replay.item_origins()[range])
                                    .filter_map(|(item, origin)| {
                                        matches!(
                                            item,
                                            ModelReplayItem::ProviderPrivateAssistant { .. }
                                        )
                                        .then_some(origin.original().binding_epoch())
                                    })
                                    .collect(),
                            })
                        })
                        .collect::<Option<Vec<_>>>()?;
                    current_context = Some(1);
                } else {
                    if closed_owner
                        != owner_epoch.map(|owner| (owner, BindingCloseReason::Replaced))
                        || owner_epoch.and_then(|owner: u64| owner.checked_add(1))
                            != Some(binding.epoch())
                    {
                        return None;
                    }
                    if binding.transition().mode() == TransitionMode::ExactReplay {
                        let valid_source = match (
                            binding.transition().source_anchor_sequence(),
                            binding.transition().source_checkpoint_sequence(),
                            binding.transition().source_initial_fork_sequence(),
                        ) {
                            (Some(sequence), None, None) => source_anchor == Some(sequence),
                            (None, Some(sequence), None) => source_checkpoint == Some(sequence),
                            (None, None, Some(sequence)) => {
                                sequence == seed_sequence
                                    && !accepted_since_seed
                                    && source_checkpoint.is_none()
                            },
                            _ => false,
                        };
                        if !valid_source {
                            return None;
                        }
                        groups = transfer_context_groups(groups, entry.sequence());
                    } else {
                        if groups.iter().any(|group| group.fork_import.is_some()) {
                            return None;
                        }
                        groups.clear();
                    }
                }
                owner_epoch = Some(binding.epoch());
                closed_owner = None;
            },
            SemanticRecord::BackendBindingOpened(binding) => {
                if owner_epoch.is_some() {
                    if binding.transition().mode() == TransitionMode::ExactReplay {
                        groups = transfer_context_groups(groups, entry.sequence());
                    } else {
                        groups.clear();
                    }
                } else {
                    current_context = Some(1);
                }
                owner_epoch = Some(binding.epoch());
                closed_owner = None;
            },
            SemanticRecord::ContextCheckpoint(checkpoint)
                if Some(checkpoint.epoch()) == owner_epoch
                    || (registered.is_none()
                        && checkpoint.epoch() == epoch
                        && checkpoint.successor_context_epoch() == context_epoch) =>
            {
                let root = checkpoint.replay_root().ok()?;
                if owner_epoch.is_some() {
                    if current_context != Some(checkpoint.previous_context_epoch()) {
                        return None;
                    }
                    current_context = Some(checkpoint.successor_context_epoch());
                    source_checkpoint = Some(entry.sequence());
                    source_anchor = None;
                }
                groups.clear();
                let retained_image_losses = checkpoint
                    .retained_groups()
                    .iter()
                    .enumerate()
                    .map(|(group_index, group)| {
                        ContextImageLoss::for_items(
                            group.items(),
                            checkpoint.successor_context_epoch(),
                            |item_index, part_index| ContextImageSource::RetainedCheckpoint {
                                sequence: entry.sequence().get(),
                                group_index: group_index as u32,
                                item_index,
                                part_index,
                            },
                        )
                        .ok()
                    })
                    .collect::<Option<Vec<_>>>()?;
                if checkpoint
                    .retained_groups()
                    .iter()
                    .any(|group| group.fork_import().is_some())
                {
                    groups.push(local_context_group(
                        entry.sequence(),
                        root.items()[..1].to_vec(),
                        Vec::new(),
                    ));
                    let mut local_tail = Vec::new();
                    let mut local_losses = Vec::new();
                    for (group_index, retained) in checkpoint.retained_groups().iter().enumerate() {
                        if let Some(import) = retained.fork_import() {
                            if !local_tail.is_empty() {
                                return None;
                            }
                            groups.push(ContextSourceGroup {
                                first_sequence: import.0,
                                last_sequence: import.0,
                                replay_sequence: import.0,
                                items: retained.items().to_vec(),
                                fork_import: Some(import),
                                private_epochs: retained.private_epochs().to_vec(),
                                image_losses: retained_image_losses[group_index].clone(),
                            });
                        } else {
                            local_tail.extend_from_slice(retained.items());
                            local_losses.extend(retained_image_losses[group_index].iter().cloned());
                        }
                    }
                    if !local_tail.is_empty() {
                        groups.push(local_context_group(
                            entry.sequence(),
                            local_tail,
                            local_losses,
                        ));
                    }
                    if !groups
                        .iter()
                        .flat_map(|group| &group.items)
                        .eq(root.items())
                    {
                        return None;
                    }
                } else {
                    groups.push(local_context_group(
                        entry.sequence(),
                        root.items().to_vec(),
                        retained_image_losses.into_iter().flatten().collect(),
                    ));
                }
            },
            SemanticRecord::ContinuationAnchor(anchor)
                if (Some(anchor.epoch()) == owner_epoch
                    && anchor.context_epoch() == current_context)
                    || (registered.is_none()
                        && anchor.epoch() == epoch
                        && anchor.context_epoch() == Some(context_epoch)) =>
            {
                source_anchor = Some(entry.sequence());
                source_checkpoint = None;
                let outcome = entries.iter().find(|candidate| {
                    candidate.sequence() == anchor.resumable_outcome_sequence()
                })?;
                let SemanticRecord::BackendResumableOutcome(outcome) = outcome.record() else {
                    return None;
                };
                let replay_sequence = outcome.replay_delta_sequence()?;
                let replay_index = entries
                    .iter()
                    .position(|candidate| candidate.sequence() == replay_sequence)?;
                let SemanticRecord::ModelReplayDelta(delta) = entries[replay_index].record() else {
                    return None;
                };
                let first = replay_index
                    .checked_sub(1)
                    .and_then(|position| entries.get(position))?;
                if !matches!(
                    first.record(),
                    SemanticRecord::EventCommitted(AgentEvent::TurnFinished {
                        outcome: TurnOutcome::Completed,
                        ..
                    })
                ) || index <= replay_index
                {
                    return None;
                }
                groups.push(ContextSourceGroup {
                    first_sequence: first.sequence(),
                    last_sequence: anchor.journal_boundary(),
                    replay_sequence,
                    image_losses: ContextImageLoss::for_items(
                        delta.delta().items(),
                        delta.context_epoch()?,
                        |item_index, part_index| ContextImageSource::ReplayDelta {
                            sequence: replay_sequence.get(),
                            item_index,
                            part_index,
                        },
                    )
                    .ok()?,
                    items: delta.delta().items().to_vec(),
                    fork_import: None,
                    private_epochs: Vec::new(),
                });
            },
            _ => {},
        }
    }
    if registered.is_some()
        && (owner_epoch != Some(epoch)
            || current_context != Some(context_epoch)
            || closed_owner.is_some())
    {
        return None;
    }
    (!groups.is_empty()).then_some(groups)
}

pub(super) fn transfer_context_groups(
    groups: Vec<ContextSourceGroup>,
    sequence: JournalSequence,
) -> Vec<ContextSourceGroup> {
    let mut transferred = Vec::new();
    let mut local = Vec::new();
    let mut local_losses = Vec::new();
    for group in groups {
        if group.fork_import.is_some() {
            if !local.is_empty() {
                transferred.push(local_context_group(
                    sequence,
                    mem::take(&mut local),
                    mem::take(&mut local_losses),
                ));
            }
            transferred.push(group);
        } else {
            local.extend(group.items);
            local_losses.extend(group.image_losses);
        }
    }
    if !local.is_empty() {
        transferred.push(local_context_group(sequence, local, local_losses));
    }
    transferred
}

fn local_context_group(
    sequence: JournalSequence,
    items: Vec<ModelReplayItem>,
    image_losses: Vec<ContextImageLoss>,
) -> ContextSourceGroup {
    ContextSourceGroup {
        first_sequence: sequence,
        last_sequence: sequence,
        replay_sequence: sequence,
        items,
        fork_import: None,
        private_epochs: Vec::new(),
        image_losses,
    }
}
