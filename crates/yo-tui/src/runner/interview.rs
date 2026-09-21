//! The TuiSession-owned working-copy controller. The runtime remains the sole Journal writer.
use std::{
    collections::HashSet,
    fmt,
    time::{Duration, Instant},
};

use yo_core::{
    ActivityKind, ActivityRequestRef, ActivityResponse, AgentEvent, SecretInput, SessionId,
    SubmissionOutcome, TranscriptReader, TranscriptRecord,
    interview::{
        Answer, CapturedInterview, InterviewCatalog, InterviewError, InterviewQuestion,
        InterviewRepository, NewConversation, PREVIEW_LIMIT, SecretRecoveryDestination, Submission,
        WorkingCopy,
    },
};

/// Read-only host access to genuine stored captures and accepted first-Turn receipts.
pub trait InterviewHistoryHost: Send {
    fn resolve(&mut self, source: ActivityRequestRef) -> Result<InterviewCatalog, InterviewError>;
    fn historical_interviews(
        &mut self,
        session: SessionId,
    ) -> Result<InterviewCatalog, InterviewError>;
    fn validate_submission(&mut self, copy: &WorkingCopy) -> Result<(), InterviewError>;
}

pub(super) struct InterviewController {
    repository: InterviewRepository,
    host: Box<dyn InterviewHistoryHost>,
    selected_session: SessionId,
    historical_requests: HashSet<ActivityRequestRef>,
    resume_liveness_unavailable: bool,
    startup_warning: Option<String>,
    live_requests: HashSet<ActivityRequestRef>,
    offered_live: HashSet<ActivityRequestRef>,
    discarded_interviews: HashSet<ActivityRequestRef>,
    startup_checked: bool,
    live: InterviewCatalog,
    source: InterviewCatalog,
    copy: Option<WorkingCopy>,
    expected: Option<u64>,
    editing: bool,
    preview: Option<String>,
    dirty: bool,
    next_save: Option<Instant>,
    automatic_save: bool,
    status: String,
    pending: Option<PendingInterview>,
    unconfirmed: Option<NewConversation>,
    recovery_destination: Option<SecretRecoveryDestination>,
    next_recovery_maintenance: Instant,
}
struct PendingInterview {
    intent: NewConversation,
    reader: TranscriptReader,
    copy: WorkingCopy,
    source: InterviewCatalog,
    expected: Option<u64>,
    receipt_seen: bool,
}
impl fmt::Debug for InterviewController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InterviewController")
            .field("editing", &self.editing)
            .field("dirty", &self.dirty)
            .field(
                "unconfirmed",
                &self
                    .unconfirmed
                    .as_ref()
                    .map(|intent| intent.submission.id()),
            )
            .finish_non_exhaustive()
    }
}
pub(super) struct InterviewCommand {
    pub document: String,
    pub editor: Option<String>,
    pub conversation: Option<NewConversation>,
}
fn error(message: &str) -> InterviewError {
    InterviewError::Invalid(message.into())
}
impl InterviewController {
    pub(super) fn live_draft(
        &self,
        activity: yo_core::ActivityRef,
    ) -> Option<(Option<u32>, String)> {
        if self.editing {
            return None;
        }
        let capture = self.live.interviews().iter().find(|capture| {
            capture.interview.activity().turn() == activity.turn()
                && self
                    .copy
                    .as_ref()
                    .is_some_and(|copy| copy.source().0 == capture.interview)
        })?;
        let copy = self.copy.as_ref()?;
        let answer = copy
            .answers
            .iter()
            .find(|answer| answer.question_id == capture.current_question_id)?;
        if answer.option_id.is_none() && answer.text.is_empty() && answer.notes.is_empty() {
            return None;
        }
        Some((
            answer.option_id.as_ref().and_then(|id| id.parse().ok()),
            if answer.option_id.is_some() {
                answer.notes.clone()
            } else {
                answer.text.clone()
            },
        ))
    }
    pub(super) fn new(
        repository: InterviewRepository,
        mut host: Box<dyn InterviewHistoryHost>,
        selected_session: SessionId,
        is_resume: bool,
    ) -> Self {
        let (historical_requests, startup_warning, resume_liveness_unavailable) = if is_resume {
            match host.historical_interviews(selected_session) {
                Ok(catalog) => (
                    catalog
                        .interviews()
                        .iter()
                        .map(|capture| capture.interview)
                        .collect(),
                    None,
                    false,
                ),
                Err(error) => (
                    HashSet::new(),
                    Some(format!(
                        "Interview draft liveness unavailable after resume: {error}"
                    )),
                    true,
                ),
            }
        } else {
            (HashSet::new(), None, false)
        };
        let mut controller = Self {
            repository,
            host,
            selected_session,
            historical_requests,
            resume_liveness_unavailable,
            startup_warning,
            live_requests: HashSet::new(),
            offered_live: HashSet::new(),
            discarded_interviews: HashSet::new(),
            startup_checked: false,
            live: InterviewCatalog::default(),
            source: InterviewCatalog::default(),
            copy: None,
            expected: None,
            editing: false,
            preview: None,
            dirty: false,
            next_save: None,
            automatic_save: true,
            status: "No interview copy selected".into(),
            pending: None,
            unconfirmed: None,
            recovery_destination: None,
            next_recovery_maintenance: Instant::now() + Duration::from_secs(60),
        };
        if let Some(notice) = controller.maintain_recovery() {
            controller.status = notice;
        }
        controller
    }

    pub(super) fn set_recovery_destination(&mut self, destination: SecretRecoveryDestination) {
        self.recovery_destination = Some(destination);
    }

    pub(super) fn recovery_boundary(&self) -> Result<String, InterviewError> {
        if self
            .copy
            .as_ref()
            .is_some_and(WorkingCopy::is_contextual_draft)
        {
            return Err(error(
                "saved secret recovery is not available for the current contextual draft",
            ));
        }
        if self.recovery_destination.is_none() {
            return Err(error(
                "secret recovery is unsupported because the live destination lacks complete authenticated account evidence",
            ));
        }
        self.repository
            .recovery_boundary()
            .ok_or_else(|| error("secret recovery storage is unavailable"))
    }

    pub(super) fn recovery_available(
        &self,
        request: ActivityRequestRef,
    ) -> Result<bool, InterviewError> {
        let Some(destination) = self.recovery_destination.as_ref() else {
            return Ok(false);
        };
        let Some((live, question_id)) = self.matching_live_secret(request)? else {
            return Ok(false);
        };
        let copy = self
            .copy
            .as_ref()
            .ok_or_else(|| error("select a saved interview copy before recovery"))?;
        self.repository
            .recovery_available(copy, &self.source, &live, &question_id, destination)
    }

    pub(super) fn store_secret_recovery(
        &mut self,
        request: ActivityRequestRef,
        secret: &SecretInput,
    ) -> Result<(), InterviewError> {
        self.save()?;
        let destination = self
            .recovery_destination
            .as_ref()
            .ok_or_else(|| error(
                "secret recovery is unsupported because the live destination lacks complete authenticated account evidence",
            ))?;
        let (_, question_id) = self.matching_live_secret(request)?.ok_or_else(|| {
            error("the live secret request does not match the selected saved copy")
        })?;
        let copy = self
            .copy
            .as_ref()
            .ok_or_else(|| error("select a saved interview copy before enabling recovery"))?;
        let published = self.repository.store_secret_recovery(
            copy,
            self.expected,
            &self.source,
            &question_id,
            destination,
            secret,
        )?;
        self.expected = Some(published.generation);
        self.copy = Some(published);
        self.dirty = false;
        self.next_save = None;
        self.status = "Saved · encrypted secret recovery available".into();
        Ok(())
    }

    pub(super) fn recover_secret(
        &self,
        request: ActivityRequestRef,
    ) -> Result<SecretInput, InterviewError> {
        let destination = self
            .recovery_destination
            .as_ref()
            .ok_or_else(|| error(
                "secret recovery is unsupported because the live destination lacks complete authenticated account evidence",
            ))?;
        let (live, question_id) = self.matching_live_secret(request)?.ok_or_else(|| {
            error("the live secret request does not match the selected saved copy")
        })?;
        let copy = self
            .copy
            .as_ref()
            .ok_or_else(|| error("select a saved interview copy before recovery"))?;
        self.repository
            .recover_secret(copy, &self.source, &live, &question_id, destination)
    }

    fn matching_live_secret(
        &self,
        request: ActivityRequestRef,
    ) -> Result<Option<(CapturedInterview, String)>, InterviewError> {
        let copy = self
            .copy
            .as_ref()
            .ok_or_else(|| error("select a saved interview copy before recovery"))?;
        let source = copy.validate(&self.source)?;
        let source_fingerprint = source.public_batch_fingerprint()?;
        let (live, index) = self
            .live
            .question_for_request(request)
            .ok_or_else(|| error("the live secret request is unavailable"))?;
        let question = live
            .questions
            .get(index)
            .ok_or_else(|| error("the live secret question is unavailable"))?;
        if !question.is_secret
            || source_fingerprint != live.public_batch_fingerprint()?
            || !source
                .questions
                .iter()
                .any(|candidate| candidate.id == question.id && candidate.is_secret)
        {
            return Ok(None);
        }
        let matches = self
            .live
            .interviews()
            .iter()
            .filter(|candidate| {
                candidate
                    .public_batch_fingerprint()
                    .is_ok_and(|fingerprint| fingerprint == source_fingerprint)
                    && candidate
                        .questions
                        .iter()
                        .any(|candidate| candidate.id == question.id && candidate.is_secret)
            })
            .count();
        if matches != 1 {
            return Err(error(
                "secret recovery is unavailable because the matching live request is ambiguous",
            ));
        }
        Ok(Some((live.clone(), question.id.clone())))
    }

    pub(super) fn forget_secret_recovery(
        &mut self,
        request: Option<ActivityRequestRef>,
    ) -> Result<Option<String>, InterviewError> {
        self.save()?;
        let copy = self
            .copy
            .as_ref()
            .ok_or_else(|| error("select a saved interview copy before forgetting recovery"))?;
        let question_id = if let Some(request) = request {
            let (live, index) = self
                .live
                .question_for_request(request)
                .ok_or_else(|| error("the live secret request is unavailable"))?;
            live.questions
                .get(index)
                .map(|question| question.id.clone())
                .ok_or_else(|| error("the live secret question is unavailable"))?
        } else {
            copy.current_question_id.clone()
        };
        let update = self.repository.forget_secret_recovery(
            copy,
            self.expected,
            &self.source,
            &question_id,
        )?;
        self.expected = Some(update.copy.generation);
        self.copy = Some(update.copy);
        self.dirty = false;
        self.next_save = None;
        self.status = "Saved · secret recovery forgotten".into();
        Ok(update.cleanup_warning)
    }
    pub(super) fn editing_text(&self) -> String {
        self.preview.clone().unwrap_or_else(|| self.answer_text())
    }
    pub(super) fn is_editing(&self) -> bool {
        self.editing
    }
    pub(super) fn is_editing_secret(&self) -> bool {
        if !self.editing {
            return false;
        }
        let Some(copy) = &self.copy else {
            return false;
        };
        self.source
            .find(copy.source().0, copy.source().1)
            .and_then(|capture| {
                capture
                    .questions
                    .iter()
                    .find(|question| question.id == copy.current_question_id)
            })
            .is_some_and(|question| question.is_secret)
    }
    pub(super) fn observe(&mut self, record: &TranscriptRecord) -> Option<String> {
        self.live.observe_committed(record);
        match record {
            TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted {
                activity,
                kind: ActivityKind::UserInputRequest { request_id },
            }) if activity.turn().session_id() == self.selected_session => {
                let request = ActivityRequestRef::new(*activity, *request_id);
                if !self.resume_liveness_unavailable && !self.historical_requests.contains(&request)
                {
                    self.live_requests.insert(request);
                }
            },
            TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished { activity, .. }) => {
                self.live_requests
                    .retain(|request| request.activity() != *activity);
                self.offered_live
                    .retain(|request| request.activity() != *activity);
            },
            TranscriptRecord::EventCommitted(AgentEvent::TurnFinished { turn, .. }) => {
                self.live_requests
                    .retain(|request| request.activity().turn() != *turn);
                self.offered_live
                    .retain(|request| request.activity().turn() != *turn);
                self.discarded_interviews
                    .retain(|interview| interview.activity().turn() != *turn);
            },
            _ => {},
        }
        if self.live.interviews().iter().any(|capture| {
            capture.submitted.is_some()
                && self
                    .copy
                    .as_ref()
                    .is_some_and(|copy| copy.source().0 == capture.interview)
        }) {
            return self.cleanup_sealed_selected().or_else(|| {
                self.copy
                    .is_some()
                    .then(|| "Submitted interview draft cleanup is pending".into())
            });
        }
        let relevant = match record {
            TranscriptRecord::EventCommitted(
                AgentEvent::ActivityUpdated { activity, .. }
                | AgentEvent::ActivityFinished { activity, .. },
            ) => self.live.is_interview_activity(*activity),
            _ => false,
        };
        if !relevant || self.editing {
            return None;
        }
        let visible = self.live.interviews().last()?.clone();
        // Only the real stored Journal can supply a recoverable source and final seal.
        let durable = self.host.resolve(visible.interview).and_then(|catalog| {
            let capture = catalog
                .find(visible.interview, &visible.revision)
                .cloned()
                .ok_or_else(|| error("complete capture is not durable"))?;
            Ok((catalog, capture))
        });
        let (source, latest) = match durable {
            Ok(value) => value,
            Err(error) => {
                let status = format!("Volatile interview; complete recovery unavailable: {error}");
                if self.status == status {
                    return None;
                }
                self.status = status.clone();
                return Some(status);
            },
        };
        // A later activity event can arrive after the final seal removed the
        // selected draft. Never recreate a draft from that submitted capture.
        if latest.submitted.is_some() {
            return None;
        }
        if self.discarded_interviews.contains(&latest.interview) {
            return None;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.copy.source().0 == latest.interview)
        {
            return None;
        }
        if self
            .copy
            .as_ref()
            .is_none_or(|copy| copy.source().0 != latest.interview)
        {
            let existing = match self.existing_copy_for(&latest, &source) {
                Ok(copy) => copy,
                Err(error) => return Some(error.to_string()),
            };
            let expected = existing.as_ref().map(|copy| copy.generation);
            let copy = match existing.map_or_else(|| WorkingCopy::new_contextual(&latest), Ok) {
                Ok(copy) => copy,
                Err(error) => return Some(error.to_string()),
            };
            self.expected = expected;
            self.copy = Some(copy);
            self.source = source;
            if self.expected.is_none() {
                self.mark_dirty();
            }
        } else {
            self.source = source;
            let copy = self.copy.as_mut().expect("selected copy");
            let before = copy.clone();
            copy.current_question_id = latest.current_question_id.clone();
            if let TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished {
                outcome: yo_core::ActivityOutcome::Completed,
                activity,
            }) = record
            {
                // Only actual completed answering responses enter this projection.
                if let Some((interview, index, answer)) = self.source.completed_answer(*activity)
                    && interview == copy.source().0
                    && let Some(question) = self
                        .source
                        .find(copy.source().0, copy.source().1)
                        .and_then(|capture| capture.questions.get(index))
                {
                    copy.answers[index] = Self::project_answer(question, answer.clone());
                }
            }
            if let Some((final_request, response_activity)) = latest.submitted {
                copy.answers = match Self::project_editable_answers(&latest, &latest.answers) {
                    Ok(answers) => answers,
                    Err(error) => return Some(error.to_string()),
                };
                copy.submission = Some(Submission::ActivityResponse {
                    final_request,
                    response_activity,
                });
            }
            if *copy != before {
                self.mark_dirty();
            }
        }
        if let Some(copy) = &self.copy
            && let Some(reason) = self.source.recovery_unavailable(copy.source().0)
        {
            self.status = format!("Submission sealing unconfirmed; {reason}");
            return Some(self.status.clone());
        }
        if self.automatic_save
            && let Err(error) = self.save()
        {
            return Some(error.to_string());
        }
        let live_request = self.live_requests.iter().find(|request| {
            self.live
                .question_for_request(**request)
                .is_some_and(|(capture, _)| {
                    self.copy
                        .as_ref()
                        .is_some_and(|copy| copy.source().0 == capture.interview)
                })
        });
        if let Some(request) = live_request.copied()
            && self.offered_live.insert(request)
        {
            return Some(
                "Interview draft available here: /interview continue or /interview discard".into(),
            );
        }
        None
    }

    fn cleanup_sealed_selected(&mut self) -> Option<String> {
        let copy = self.copy.as_ref()?;
        let source = self.host.resolve(copy.source().0).ok()?;
        let capture = copy.validate(&source).ok()?;
        capture.submitted?;
        let persisted = self.repository.load(&copy.copy_id);
        let cleanup = persisted.and_then(|stored| {
            if let Some(stored) = stored {
                stored.validate(&source)?;
                self.delete_copy(&stored)?;
            }
            Ok(())
        });
        match cleanup {
            Ok(()) => {
                self.copy = None;
                self.expected = None;
                self.dirty = false;
                self.next_save = None;
                self.editing = false;
                self.preview = None;
                self.status = "Submitted interview draft removed".into();
                None
            },
            Err(error) => Some(format!(
                "Submitted interview draft cleanup is pending: {error}"
            )),
        }
    }

    fn delete_copy(&self, copy: &WorkingCopy) -> Result<(), InterviewError> {
        self.repository.delete(copy)
    }

    fn stored_copies(&self) -> Result<Vec<WorkingCopy>, InterviewError> {
        self.repository
            .list()?
            .into_iter()
            .map(|(_, result)| {
                result
                    .map_err(|_| error("Stored interview copy is unreadable; no draft was changed"))
            })
            .collect()
    }

    fn existing_copy_for(
        &self,
        capture: &CapturedInterview,
        source: &InterviewCatalog,
    ) -> Result<Option<WorkingCopy>, InterviewError> {
        let mut matches = self
            .stored_copies()?
            .into_iter()
            .filter(|copy| copy.is_contextual_draft() && copy.source().0 == capture.interview)
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err(error("multiple contextual drafts match one request"));
        }
        let copy = matches.pop();
        if let Some(copy) = &copy {
            copy.validate(source)?;
        }
        Ok(copy)
    }

    fn maintain_recovery(&mut self) -> Option<String> {
        if self.dirty || self.pending.is_some() {
            return None;
        }
        let mut notices = Vec::new();
        match self.repository.maintain_secret_recovery() {
            Ok(maintenance) => {
                if maintenance.expired_references > 0 {
                    notices.push(format!(
                        "Expired {} secret recovery reference(s)",
                        maintenance.expired_references
                    ));
                }
                if let Some(warning) = maintenance.warning {
                    notices.push(warning);
                }
            },
            Err(error) => notices.push(format!("Secret recovery maintenance unavailable: {error}")),
        }

        let copies = match self.repository.list() {
            Ok(copies) => copies,
            Err(error) => {
                notices.push(format!(
                    "Secret recovery seal reconciliation unavailable: {error}"
                ));
                return (!notices.is_empty()).then(|| notices.join("; "));
            },
        };
        let selected_id = self.copy.as_ref().map(|copy| copy.copy_id.clone());
        for (_, value) in copies {
            let Ok(mut copy) = value else {
                continue;
            };
            if !copy.has_any_secret_recovery() {
                continue;
            }
            let source = match self.host.resolve(copy.source().0) {
                Ok(source) => source,
                Err(_) => continue,
            };
            let capture = match copy.validate(&source) {
                Ok(capture) => capture.clone(),
                Err(_) => continue,
            };
            let Some((final_request, response_activity)) = capture.submitted else {
                continue;
            };
            copy.answers = match Self::project_editable_answers(&capture, &capture.answers) {
                Ok(answers) => answers,
                Err(_) => continue,
            };
            copy.current_question_id = capture.current_question_id.clone();
            copy.submission = Some(Submission::ActivityResponse {
                final_request,
                response_activity,
            });
            match self
                .repository
                .forget_all_secret_recovery(&copy, Some(copy.generation), &source)
            {
                Ok(update) => {
                    if selected_id.as_deref() == Some(update.copy.copy_id.as_str()) {
                        self.expected = Some(update.copy.generation);
                        self.copy = Some(update.copy.clone());
                        self.source = source;
                        self.editing = false;
                    }
                    notices.push("Final response seal removed secret recovery".to_owned());
                    if let Some(warning) = update.cleanup_warning {
                        notices.push(warning);
                    }
                },
                Err(error) => notices.push(format!(
                    "Final response sealed, but secret recovery cleanup is pending: {error}"
                )),
            }
        }

        if let Some(id) = selected_id
            && let Ok(Some(current)) = self.repository.load(&id)
            && self
                .copy
                .as_ref()
                .is_some_and(|copy| copy.generation != current.generation)
        {
            self.expected = Some(current.generation);
            self.copy = Some(current);
        }
        (!notices.is_empty()).then(|| notices.join("; "))
    }
    pub(super) fn retain_live_response(
        &mut self,
        request: ActivityRequestRef,
        response: ActivityResponse,
    ) -> Option<String> {
        if self.editing {
            return None;
        }
        let (capture, index) = self.live.question_for_request(request)?;
        if self.copy.as_ref()?.source().0 != capture.interview {
            return None;
        }
        let answer = match &response {
            ActivityResponse::PreviousQuestion { choice, draft } => {
                let mut answer = capture.questions[index].empty_answer();
                answer.option_id = choice.map(|value| value.to_string());
                if choice.is_some() {
                    answer.notes = draft.as_str().into();
                } else {
                    answer.text = draft.as_str().into();
                }
                answer
            },
            _ => match capture.questions[index].project_response(&response) {
                Ok(answer) => answer,
                Err(error) => return Some(error.to_string()),
            },
        };
        let answer = Self::project_answer(&capture.questions[index], answer);
        self.copy.as_mut().expect("selected copy").answers[index] = answer;
        self.mark_dirty();
        self.save().err().map(|e| e.to_string())
    }
    pub(super) fn edit_text(
        &mut self,
        text: &str,
        live_request: Option<ActivityRequestRef>,
        choice: Option<u32>,
    ) {
        if self.is_editing_secret() {
            return;
        }
        let literal = self.editing && text.starts_with("//");
        let text = if literal { &text[1..] } else { text };
        // Clearing the editor is also how users make room for an interview
        // command. Keep the last committed answer or preview during that
        // transition; explicit interview commands own deliberate resets.
        if self.editing && text.is_empty() {
            return;
        }
        if (!literal && text.starts_with('/'))
            || self.pending.as_ref().is_some_and(|pending| {
                self.copy
                    .as_ref()
                    .is_some_and(|copy| copy.copy_id == pending.copy.copy_id)
            })
        {
            return;
        }
        if self.editing && self.preview.is_some() {
            self.preview = Some(text.into());
            return;
        }
        let Some(copy) = self.copy.as_mut() else {
            return;
        };
        if copy.submission.is_some() {
            return;
        }
        let index = if self.editing {
            copy.answers
                .iter()
                .position(|a| a.question_id == copy.current_question_id)
        } else {
            live_request
                .and_then(|request| self.live.question_for_request(request))
                .filter(|(capture, _)| capture.interview == copy.source().0)
                .map(|(_, index)| index)
        };
        let Some(index) = index else {
            return;
        };
        let mut answer = copy.answers[index].clone();
        if let Some(choice) = choice {
            answer.option_id = Some(choice.to_string());
            answer.text.clear();
            answer.notes = text.into();
        } else {
            answer.option_id = None;
            answer.text = text.into();
            if self.editing
                && let Some(capture) = self.source.find(copy.source().0, copy.source().1)
                && let Ok(mut projected) = capture.questions[index]
                    .project_response(&ActivityResponse::UserInput(yo_core::UserInput::new(text)))
            {
                projected.notes = answer.notes;
                answer = projected;
            }
        }
        if copy.answers[index] != answer {
            copy.answers[index] = answer;
            self.mark_dirty();
        }
    }
    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.automatic_save = true;
        self.status = "Unsaved editable answers".into();
        if self.next_save.is_none() {
            self.next_save = Some(Instant::now() + Duration::from_millis(250));
        }
    }

    /// Project the volatile live answer view into the editable working-copy shape.
    ///
    /// A v2 capture may contain a submitted secret answer while it is still being
    /// observed.  The working copy deliberately cannot persist that answer state:
    /// every secret row is always the payload-free re-entry marker.  Keep public
    /// answers byte-for-byte intact while projecting only the secret rows.
    fn project_editable_answers(
        capture: &CapturedInterview,
        answers: &[Answer],
    ) -> Result<Vec<Answer>, InterviewError> {
        if capture.questions.len() != answers.len() {
            return Err(error("working copy answer count does not match capture"));
        }
        Ok(capture
            .questions
            .iter()
            .zip(answers)
            .map(|(question, answer)| Self::project_answer(question, answer.clone()))
            .collect())
    }

    fn project_answer(question: &InterviewQuestion, answer: Answer) -> Answer {
        if question.is_secret {
            question.empty_answer()
        } else {
            answer
        }
    }

    fn project_copy_for_save(&mut self, copy: &mut WorkingCopy) -> Result<(), InterviewError> {
        let (interview, revision) = (copy.source().0, copy.source().1.to_owned());
        let capture = self
            .source
            .find(interview, &revision)
            .ok_or_else(|| error("complete capture for working-copy save is unavailable"))?;
        copy.answers = Self::project_editable_answers(capture, &copy.answers)?;
        Ok(())
    }

    fn save(&mut self) -> Result<(), InterviewError> {
        if !self.dirty {
            return Ok(());
        }
        let Some(mut copy) = self.copy.take() else {
            return Ok(());
        };
        let projection = self.project_copy_for_save(&mut copy);
        let result =
            projection.and_then(|()| self.repository.save(&copy, self.expected, &self.source));
        self.copy = Some(copy);
        match result {
            Ok(generation) => {
                let copy = self.copy.as_mut().expect("selected copy");
                copy.generation = generation;
                self.expected = Some(generation);
                self.dirty = false;
                self.next_save = None;
                self.automatic_save = true;
                self.status = self
                    .source
                    .recovery_unavailable(copy.source().0)
                    .map(|reason| {
                        format!("Saved editable answers; submission sealing unconfirmed; {reason}")
                    })
                    .unwrap_or_else(|| "Saved".into());
                Ok(())
            },
            Err(error) => {
                self.automatic_save = false;
                self.next_save = None;
                self.status = format!("Unsaved: {error}");
                Err(error)
            },
        }
    }
    pub(super) fn tick(&mut self) -> Option<String> {
        if let Some(warning) = self.startup_warning.take() {
            return Some(warning);
        }
        if !self.startup_checked {
            self.startup_checked = true;
            if self.live_requests.is_empty() {
                match self.select_contextual() {
                    Ok(Some(_)) => {
                        return Some("Saved interview draft in this Session: /interview view or /interview discard".into());
                    },
                    Ok(None) => {},
                    Err(error) => return Some(format!("Interview draft unavailable: {error}")),
                }
            }
        }
        if self.live.interviews().iter().any(|capture| {
            capture.submitted.is_some()
                && self
                    .copy
                    .as_ref()
                    .is_some_and(|copy| copy.source().0 == capture.interview)
        }) {
            return self.cleanup_sealed_selected();
        }
        if let Some(pending) = &mut self.pending
            && !pending.receipt_seen
            && let Some((turn, sequence)) = pending
                .reader
                .accepted_initial_submission(pending.intent.submission.id())
            && turn.session_id() != pending.copy.source().0.activity().turn().session_id()
        {
            pending.copy.submission = Some(Submission::NewConversation {
                turn,
                submission_id: pending.intent.submission.id().to_string(),
                accepted_request_sequence: sequence.get(),
            });
            pending.receipt_seen = true;
            return Some(match self.save_pending() {
                Ok(()) => {
                    "Interview sent as a new conversation; accepted first Turn confirmed. Saved."
                        .into()
                },
                Err(error) => {
                    self.status = format!("Unsaved accepted first-Turn record: {error}");
                    self.status.clone()
                },
            });
        }
        if self.pending.as_ref().is_some_and(|pending| {
            !pending.receipt_seen
                && (pending
                    .reader
                    .initial_submission_terminated(pending.intent.submission.id())
                    || !matches!(
                        pending.reader.durability(),
                        yo_core::JournalDurability::Durable { .. }
                    ))
        }) {
            let pending = self.pending.take().expect("unconfirmed pending interview");
            if self
                .copy
                .as_ref()
                .is_none_or(|copy| copy.copy_id == pending.copy.copy_id)
            {
                self.preview = Some(pending.intent.preview.clone());
                self.copy = Some(pending.copy);
                self.source = pending.source;
                self.expected = pending.expected;
                self.dirty = false;
                self.next_save = None;
            } else {
                // A following live copy may contain unsaved edits; keep it selected and editable.
                self.preview = None;
            }
            self.unconfirmed = Some(pending.intent);
            self.editing = true;
            self.status = "Submission unconfirmed; immutable intent retained and preview editable. No automatic retry.".into();
            return Some(self.status.clone());
        }
        if Instant::now() >= self.next_recovery_maintenance {
            self.next_recovery_maintenance = Instant::now() + Duration::from_secs(60);
            if let Some(notice) = self.maintain_recovery() {
                self.status = notice.clone();
                return Some(notice);
            }
        }
        if self.automatic_save
            && self
                .next_save
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return self.save().err().map(|error| error.to_string());
        }
        None
    }
    fn save_pending(&mut self) -> Result<(), InterviewError> {
        let Some(pending) = &mut self.pending else {
            return Ok(());
        };
        if !pending.receipt_seen {
            return Ok(());
        }
        let generation = self
            .repository
            .save(&pending.copy, pending.expected, &pending.source)?;
        pending.copy.generation = generation;
        if self
            .copy
            .as_ref()
            .is_some_and(|copy| copy.copy_id == pending.copy.copy_id)
        {
            self.copy = Some(pending.copy.clone());
            self.expected = Some(generation);
            self.dirty = false;
            self.next_save = None;
            self.status = "Saved".into();
        }
        self.pending = None;
        Ok(())
    }
    pub(super) fn flush(&mut self) -> Option<String> {
        if let Err(error) = self.save_pending() {
            return Some(error.to_string());
        }
        self.save().err().map(|e| e.to_string())
    }
    pub(super) fn accepted_pending(&mut self, intent: NewConversation, reader: TranscriptReader) {
        if let Some(copy) = &self.copy {
            self.pending = Some(PendingInterview {
                intent,
                reader,
                copy: copy.clone(),
                source: self.source.clone(),
                expected: self.expected,
                receipt_seen: false,
            });
        }
        self.unconfirmed = None;
        self.live = InterviewCatalog::default();
        self.editing = false;
        self.status = "Submission unconfirmed; editable copy retained".into();
    }
    pub(super) fn observe_submission(&mut self, outcome: &SubmissionOutcome) -> Option<String> {
        if let SubmissionOutcome::Rejected { id, rejection } = outcome
            && self.pending.as_ref().is_some_and(|pending| {
                !pending.receipt_seen && pending.intent.submission.id() == *id
            })
        {
            self.pending = None;
            self.editing = true;
            self.status = "Submission unconfirmed; editable copy retained".into();
            return Some(format!("Interview not submitted: {}", rejection.message()));
        }
        None
    }
    fn select_contextual(&mut self) -> Result<Option<WorkingCopy>, InterviewError> {
        self.cleanup_sealed_selected();
        if let Some(copy) = &self.copy {
            let source = self.host.resolve(copy.source().0)?;
            if copy.validate(&source)?.submitted.is_some() {
                return Err(error("submitted draft cleanup is pending"));
            }
        }
        self.save()?;
        let mut copies = self
            .stored_copies()?
            .into_iter()
            .filter(|copy| {
                copy.is_contextual_draft()
                    && copy.source().0.activity().turn().session_id() == self.selected_session
            })
            .collect::<Vec<_>>();
        copies.sort_by(|left, right| left.copy_id.cmp(&right.copy_id));
        copies.sort_by_key(|copy| !self.current_request_is_live(copy));
        let mut sources = HashSet::new();
        if copies.iter().any(|copy| !sources.insert(copy.source().0)) {
            return Err(error("multiple contextual drafts match one request"));
        }
        for copy in copies {
            let source = self.host.resolve(copy.source().0)?;
            let capture = copy.validate(&source)?;
            let sealed = capture.submitted.is_some();
            if sealed || copy.submission.is_some() {
                if !sealed {
                    // A legacy new-conversation copy requires its separate durable
                    // acceptance evidence before it can be discarded.
                    self.host.validate_submission(&copy)?;
                }
                self.delete_copy(&copy)?;
                continue;
            }
            self.expected = Some(copy.generation);
            self.source = source;
            self.copy = Some(copy.clone());
            self.editing = false;
            self.preview = None;
            return Ok(Some(copy));
        }
        self.copy = None;
        self.expected = None;
        self.editing = false;
        self.preview = None;
        Ok(None)
    }

    fn current_request_is_live(&self, copy: &WorkingCopy) -> bool {
        self.live_requests.iter().any(|request| {
            self.live
                .question_for_request(*request)
                .is_some_and(|(capture, _)| {
                    capture.interview == copy.source().0
                        && capture.revision == copy.source().1
                        && capture.submitted.is_none()
                })
        })
    }

    pub(super) fn command(
        &mut self,
        argument: &str,
        _busy: bool,
    ) -> Result<InterviewCommand, InterviewError> {
        let verb = argument.trim();
        if verb == "close" {
            self.save()?;
            self.editing = false;
            return Ok(InterviewCommand {
                document: "Interview draft closed".into(),
                editor: Some(String::new()),
                conversation: None,
            });
        }
        if !matches!(verb, "" | "continue" | "view" | "discard") {
            return Err(error("Use /interview to view this Session's draft options"));
        }
        let Some(copy) = self.select_contextual()? else {
            return Ok(InterviewCommand {
                document: "No unfinished interview draft in this Session".into(),
                editor: None,
                conversation: None,
            });
        };
        let live = self.current_request_is_live(&copy);
        match verb {
            "continue" if live => {
                self.status = "Live interview draft; Enter responds to the current request".into();
                Ok(InterviewCommand {
                    document: self.document()?,
                    editor: Some(self.answer_text()),
                    conversation: None,
                })
            },
            "view" if !live => {
                self.status = "Original request ended; this draft is read-only".into();
                Ok(InterviewCommand {
                    document: self.document()?,
                    editor: None,
                    conversation: None,
                })
            },
            "discard" => {
                self.delete_copy(&copy)?;
                if live {
                    self.discarded_interviews.insert(copy.source().0);
                }
                self.copy = None;
                self.expected = None;
                self.source = InterviewCatalog::default();
                self.status = "Unsubmitted interview draft discarded".into();
                Ok(InterviewCommand {
                    document: self.status.clone(),
                    editor: Some(String::new()),
                    conversation: None,
                })
            },
            "" => Ok(InterviewCommand {
                document: format!(
                    "{}\n\n{}\n/interview discard",
                    self.document()?,
                    if live {
                        "/interview continue"
                    } else {
                        "/interview view"
                    },
                ),
                editor: None,
                conversation: None,
            }),
            _ => Err(error(if live {
                "The request is still live; use /interview continue or discard"
            } else {
                "The request has ended; use /interview view or discard"
            })),
        }
    }
    fn require_editing(&self) -> Result<(), InterviewError> {
        if !self.editing || self.pending.is_some() {
            Err(error("recover or reopen an interview copy first"))
        } else {
            Ok(())
        }
    }
    pub(super) fn local_enter(&mut self, text: &str) -> Result<InterviewCommand, InterviewError> {
        self.require_editing()?;
        if self.is_editing_secret() {
            return Err(error(
                "secret interview answers require a new live request; local editing is unavailable",
            ));
        }
        if self.preview.is_some() {
            if text.len() > PREVIEW_LIMIT {
                return Err(error("editable preview exceeds 64 KiB"));
            }
            self.preview = Some(text.into());
            return Ok(InterviewCommand {
                document: "Preview edited. Use /interview send to send it as a new conversation."
                    .into(),
                editor: Some(text.into()),
                conversation: None,
            });
        }
        let copy = self.copy.as_mut().expect("editable copy");
        let capture = copy.validate(&self.source)?;
        let index = capture
            .questions
            .iter()
            .position(|q| q.id == copy.current_question_id)
            .expect("validated question");
        let mut answer = capture.questions[index]
            .project_response(&ActivityResponse::UserInput(yo_core::UserInput::new(text)))?;
        if answer.option_id == copy.answers[index].option_id {
            answer.notes = copy.answers[index].notes.clone();
        }
        copy.answers[index] = answer;
        self.mark_dirty();
        self.save()?;
        self.command("next", false)
    }
    fn answer_text(&self) -> String {
        self.copy
            .as_ref()
            .and_then(|c| {
                c.answers
                    .iter()
                    .find(|a| a.question_id == c.current_question_id)
            })
            .map(|a| {
                if a.is_secret() {
                    return String::new();
                }
                a.option_id.clone().unwrap_or_else(|| {
                    if a.text.starts_with('/') {
                        format!("/{}", a.text)
                    } else {
                        a.text.clone()
                    }
                })
            })
            .unwrap_or_default()
    }
    fn document(&self) -> Result<String, InterviewError> {
        let Some(copy) = &self.copy else {
            return Ok(self.status.clone());
        };
        let capture = copy.validate(&self.source)?;
        let index = capture
            .questions
            .iter()
            .position(|q| q.id == copy.current_question_id)
            .expect("validated current question");
        let q = &capture.questions[index];
        let a = &copy.answers[index];
        let mut text = format!(
            "Interview draft · {}\n\nQuestion {} of {}\n{}\n",
            self.status,
            index + 1,
            capture.questions.len(),
            q.prompt,
        );
        if q.is_secret {
            let state = match self.repository.stored_recovery_available(copy, &q.id) {
                Ok(true) => "secret recovery stored; live authentication required",
                Ok(false) if copy.has_secret_recovery(&q.id) => "secret recovery unavailable",
                Err(_) => "secret recovery unavailable",
                Ok(false) => "secret re-entry required",
            };
            text.push_str(&format!("\nAnswer: [{state}]\nNotes: \n\nSecret answers require a matching live request. Esc: close."));
        } else {
            text.push_str(&format!("\nAnswer: {}\nNotes: {}\n\nThis draft belongs only to its original Session and request.",a.option_id.as_deref().unwrap_or(&a.text),a.notes));
        }
        Ok(text)
    }
}
