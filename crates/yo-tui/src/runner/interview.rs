//! The TuiSession-owned working-copy controller. The runtime remains the sole Journal writer.
use std::{
    fmt,
    time::{Duration, Instant},
};

use yo_core::{
    ActivityRequestRef, ActivityResponse, SubmissionOutcome, TranscriptReader, TranscriptRecord,
    interview::{
        InterviewCatalog, InterviewError, InterviewRepository, NewConversation, PREVIEW_LIMIT,
        Submission, WorkingCopy,
    },
};

/// Read-only host access to genuine stored captures and accepted first-Turn receipts.
pub trait InterviewHistoryHost: Send {
    fn resolve(&mut self, source: ActivityRequestRef) -> Result<InterviewCatalog, InterviewError>;
    fn validate_submission(&mut self, copy: &WorkingCopy) -> Result<(), InterviewError>;
}

pub(super) struct InterviewController {
    repository: InterviewRepository,
    host: Box<dyn InterviewHistoryHost>,
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
        host: Box<dyn InterviewHistoryHost>,
    ) -> Self {
        Self {
            repository,
            host,
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
        }
    }
    pub(super) fn editing_text(&self) -> String {
        self.preview.clone().unwrap_or_else(|| self.answer_text())
    }
    pub(super) fn is_editing(&self) -> bool {
        self.editing
    }
    pub(super) fn observe(&mut self, record: &TranscriptRecord) -> Option<String> {
        self.live.observe_committed(record);
        let relevant = match record {
            TranscriptRecord::EventCommitted(
                yo_core::AgentEvent::ActivityUpdated { activity, .. }
                | yo_core::AgentEvent::ActivityFinished { activity, .. },
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
            let copy = match WorkingCopy::new(&latest) {
                Ok(copy) => copy,
                Err(error) => return Some(error.to_string()),
            };
            self.copy = Some(copy);
            self.expected = None;
            self.source = source;
            self.mark_dirty();
        } else {
            self.source = source;
            let copy = self.copy.as_mut().expect("selected copy");
            let before = copy.clone();
            copy.current_question_id = latest.current_question_id.clone();
            if let TranscriptRecord::EventCommitted(yo_core::AgentEvent::ActivityFinished {
                outcome: yo_core::ActivityOutcome::Completed,
                activity,
            }) = record
            {
                // Only actual completed answering responses enter this projection.
                if let Some((interview, index, answer)) = self.source.completed_answer(*activity)
                    && interview == copy.source().0
                {
                    copy.answers[index] = answer.clone();
                }
            }
            if let Some((final_request, response_activity)) = latest.submitted {
                copy.answers = latest.answers;
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
        if self.automatic_save {
            self.save().err().map(|error| error.to_string())
        } else {
            None
        }
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
        let literal = self.editing && text.starts_with("//");
        let text = if literal { &text[1..] } else { text };
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
    fn save(&mut self) -> Result<(), InterviewError> {
        if !self.dirty {
            return Ok(());
        }
        let Some(copy) = self.copy.as_mut() else {
            return Ok(());
        };
        match self.repository.save(copy, self.expected, &self.source) {
            Ok(generation) => {
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
    pub(super) fn command(
        &mut self,
        argument: &str,
        busy: bool,
    ) -> Result<InterviewCommand, InterviewError> {
        let (verb, rest) = argument
            .trim()
            .split_once(' ')
            .unwrap_or((argument.trim(), ""));
        let mut editor = None;
        let mut conversation = None;
        match verb {
            "" | "list" => {
                let mut document = String::from("Interview copies\n\n");
                for (id, value) in self.repository.list()? {
                    document.push_str(&format!(
                        "{id}: {}\n",
                        match value {
                            Ok(copy) => {
                                let valid = self.host.resolve(copy.source().0).and_then(|source| {
                                    copy.validate(&source)?;
                                    self.host.validate_submission(&copy)
                                });
                                match valid {
                                    Ok(()) if copy.submission.is_some() => {
                                        "submitted; use reopen".into()
                                    },
                                    Ok(()) => "saved editable copy; use recover".into(),
                                    Err(error) => format!("unavailable: {error}"),
                                }
                            },
                            Err(e) => format!("unavailable: {e}"),
                        }
                    ));
                }
                document.push_str("\n/interview recover <copy UUID> · /interview reopen <copy UUID>\n/interview next · previous · option <number> · notes <text> · context <text> · preview · send · save · close\nType an answer and press Enter to keep it locally. Sending requires /interview send.\n");
                return Ok(InterviewCommand {
                    document,
                    editor: None,
                    conversation: None,
                });
            },
            "recover" | "reopen" => {
                if self.pending.is_some() {
                    return Err(error(
                        "New-conversation confirmation is pending; the exact copy and preview are retained.",
                    ));
                }
                self.save()?;
                let stored = self
                    .repository
                    .load(rest.trim())?
                    .ok_or_else(|| error("interview copy not found"))?;
                let source = self.host.resolve(stored.source().0)?;
                stored.validate(&source)?;
                self.host.validate_submission(&stored)?;
                if verb == "recover" && stored.submission.is_some() {
                    return Err(error(
                        "submitted copies must be deliberately reopened with /interview reopen <UUID>",
                    ));
                }
                let copy = if verb == "reopen" {
                    stored.reopen()?
                } else {
                    stored.clone()
                };
                self.expected = if verb == "reopen" {
                    None
                } else {
                    Some(copy.generation)
                };
                self.source = source;
                self.copy = Some(copy);
                self.editing = true;
                self.preview = None;
                self.pending = None;
                if verb == "reopen" {
                    self.mark_dirty();
                    self.save()?;
                } else {
                    self.status = "Saved · recovered editable copy; submission unconfirmed".into();
                }
                editor = Some(self.answer_text());
            },
            "next" | "previous" => {
                self.require_editing()?;
                self.save()?;
                let copy = self.copy.as_mut().expect("editable copy");
                let current = copy
                    .answers
                    .iter()
                    .position(|a| a.question_id == copy.current_question_id)
                    .ok_or_else(|| error("invalid current question"))?;
                let next = if verb == "next" {
                    (current + 1).min(copy.answers.len() - 1)
                } else {
                    current.saturating_sub(1)
                };
                copy.current_question_id = copy.answers[next].question_id.clone();
                self.preview = None;
                self.mark_dirty();
                self.save()?;
                editor = Some(self.answer_text());
            },
            "option" | "notes" | "context" => {
                self.require_editing()?;
                let copy = self.copy.as_mut().expect("editable copy");
                let answer = copy
                    .answers
                    .iter_mut()
                    .find(|a| a.question_id == copy.current_question_id)
                    .expect("validated current question");
                match verb {
                    "option" => {
                        answer.option_id = if rest.trim() == "none" {
                            None
                        } else {
                            Some(rest.trim().into())
                        };
                        answer.text.clear();
                    },
                    "notes" => answer.notes = rest.into(),
                    _ => copy.context = rest.into(),
                }
                self.preview = None;
                self.mark_dirty();
                self.save()?;
                editor = Some(self.answer_text());
            },
            "preview" => {
                self.require_editing()?;
                self.save()?;
                let text = self
                    .copy
                    .as_ref()
                    .expect("editable copy")
                    .preview(&self.source)?;
                self.preview = Some(text.clone());
                editor = Some(text);
            },
            "send" => {
                self.require_editing()?;
                if busy {
                    return Err(error(
                        "Session is busy; the interview copy is retained. Send after the active Turn finishes.",
                    ));
                }
                self.save()?;
                let mut intent = self
                    .copy
                    .as_ref()
                    .expect("editable copy")
                    .new_conversation(&self.source)?;
                if let Some(preview) = &self.preview {
                    if preview.len() > PREVIEW_LIMIT {
                        return Err(error(
                            "editable preview exceeds 64 KiB; no text was truncated",
                        ));
                    }
                    intent.preview = preview.clone();
                    intent.submission = yo_core::InputSubmission::new(
                        intent.submission.id(),
                        yo_core::UserInput::new(preview.clone()),
                    );
                }
                conversation = Some(intent);
            },
            "save" => {
                self.save_pending()?;
                self.save()?;
                if self.editing {
                    editor = Some(self.preview.clone().unwrap_or_else(|| self.answer_text()));
                }
            },
            "close" => {
                self.save()?;
                self.editing = false;
                self.preview = None;
                editor = Some(String::new());
            },
            _ => return Err(error("Unknown interview command; use /interview list")),
        }
        let document = if self.preview.is_some() {
            format!(
                "Interview preview · {}\n\nEdit the plain text. /interview send explicitly sends it as a new conversation.\n\n{}",
                self.status,
                self.preview.as_deref().unwrap_or_default()
            )
        } else {
            self.document()?
        };
        Ok(InterviewCommand {
            document,
            editor,
            conversation,
        })
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
            "Interview copy {} · {}\n\nQuestion {} of {}\n{}\n{}\n",
            copy.copy_id,
            self.status,
            index + 1,
            capture.questions.len(),
            q.prompt,
            q.question
        );
        for o in &q.options {
            text.push_str(&format!("{}. {} — {}\n", o.id, o.label, o.description));
        }
        text.push_str(&format!("\nAnswer: {}\nNotes: {}\n\nEdit locally; /interview preview then /interview send starts a new conversation.",a.option_id.as_deref().unwrap_or(&a.text),a.notes));
        Ok(text)
    }
}
