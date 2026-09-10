use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{self, Cursor, Read},
    num::NonZeroU64,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageFormat, Rgb, RgbImage, imageops::rotate270};
use serde_json::json;
use yo_core::{
    ActivityApproval, ActivityDocument, ActivityId, ActivityKind, ActivityNotice, ActivityOutcome,
    ActivityPlan, ActivityQuestion, ActivityReasoning, ActivityRef, ActivityRequestRef,
    ActivitySummary, ActivityUpdate, AgentCommand, AgentControlOutcome, AgentEvent, ApprovalChoice,
    ApprovalDecision, Failure, MessageContent, NoticeLevel, PlanStep, PlanStepStatus,
    QuestionChoice, RequestId, SessionId, SubmissionOutcome, SubmissionRejection,
    SubmissionRejectionKind, SummaryKind, ToolOutput, TranscriptRecord, TurnId, TurnOutcome,
    TurnRef,
};

use super::TuiDocument;
use crate::runner::{
    AgentAction, AgentConnection, AgentPoll, DispatchOutcome, PendingDispatch, TuiStatusLine,
};

#[derive(Debug)]
pub(in crate::runner) struct TestAgent {
    ready: VecDeque<AgentPoll>,
    stream: VecDeque<AgentEvent>,
    active: Option<ActivityRef>,
    session: SessionId,
    turns: u64,
    next: Instant,
    armed: bool,
    request: Option<PreviewRequest>,
    interview_drafts: [Option<(Option<u32>, String)>; 2],
    interview_sequence: u64,
}

#[derive(Debug)]
enum PreviewRequest {
    Approval(ActivityRequestRef, PreviewApproval),
    Interview {
        request: ActivityRequestRef,
        first_answer: Option<String>,
        first: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewApproval {
    Plain,
    Scopes,
    FileChange,
}

impl PreviewApproval {
    fn profile(self) -> Option<ActivityApproval> {
        match self {
            Self::Plain => None,
            Self::Scopes => Some(approval_profile()),
            Self::FileChange => Some(ActivityApproval {
                related_change: Some(2),
                plain_text: "Review the proposed file changes with /changes.\nOffline preview: no files or permissions are changed.".into(),
                choices: vec![
                    ApprovalChoice { label:"Approve request".into(), description:"Accept this offline proposal".into(), enabled:true },
                    ApprovalChoice { label:"Decline".into(), description:"Leave the proposal unapplied".into(), enabled:true },
                ],
                decline_choice:Some(2),
            }),
        }
    }
}

impl TestAgent {
    pub(in crate::runner) fn new() -> Self {
        Self {
            ready: VecDeque::new(),
            stream: VecDeque::new(),
            active: None,
            session: "01890f00-0000-7000-8000-000000000001".parse().unwrap(),
            turns: 0,
            next: Instant::now(),
            armed: false,
            request: None,
            interview_drafts: [None, None],
            interview_sequence: 100,
        }
    }

    fn event(&mut self, event: AgentEvent) {
        self.ready
            .push_back(AgentPoll::Record(TranscriptRecord::EventCommitted(event)));
    }

    pub(in crate::runner) fn next_deadline(&self) -> Option<Instant> {
        if !self.ready.is_empty() {
            Some(Instant::now())
        } else if !self.stream.is_empty() {
            Some(self.next)
        } else {
            None
        }
    }

    fn ask(&mut self, activity: ActivityRef, kind: ActivityKind, text: &str) {
        self.active = Some(activity);
        self.event(AgentEvent::ActivityStarted { activity, kind });
        self.event(AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(text.to_owned()),
        });
    }

    fn next_interview_activity(&mut self, turn: TurnRef) -> Result<ActivityRef, io::Error> {
        self.interview_sequence = self
            .interview_sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("preview question identity exhausted"))?;
        Ok(ActivityRef::new(
            turn,
            ActivityId::new(NonZeroU64::new(self.interview_sequence).unwrap()),
        ))
    }

    fn show_interview(
        &mut self,
        turn: TurnRef,
        first: bool,
        first_answer: Option<String>,
    ) -> Result<(), io::Error> {
        let activity = self.next_interview_activity(turn)?;
        let request_id = RequestId::new(activity.activity_id().get());
        let mut question = ActivityQuestion::from_snapshot(&interview_prompt(first))
            .expect("structured preview question");
        question.previous_question = !first;
        if let Some((choice, draft)) = &self.interview_drafts[usize::from(!first)] {
            question.draft_choice = *choice;
            question.draft = Some(draft.clone());
        }
        let text = question
            .to_snapshot()
            .ok_or_else(|| io::Error::other("preview draft exceeds question display limit"))?;
        self.request = Some(PreviewRequest::Interview {
            request: ActivityRequestRef::new(activity, request_id),
            first_answer,
            first,
        });
        self.ask(
            activity,
            ActivityKind::UserInputRequest { request_id },
            &text,
        );
        Ok(())
    }

    fn answer_interview(
        &mut self,
        request: ActivityRequestRef,
        input: String,
        choice: Option<u32>,
    ) -> Result<(), io::Error> {
        let Some(PreviewRequest::Interview {
            request: current,
            first,
            first_answer,
        }) = &self.request
        else {
            return Ok(());
        };
        if *current != request {
            return Ok(());
        }
        let first = *first;
        let first_answer = first_answer.clone();
        let question = ActivityQuestion::from_snapshot(&interview_prompt(first))
            .expect("structured preview question");
        let resolved = match choice {
            Some(choice) => {
                let selected = choice
                    .checked_sub(1)
                    .and_then(|index| question.choices.get(index as usize))
                    .ok_or_else(|| io::Error::other("preview choice is unavailable"))?;
                if input.trim().is_empty() {
                    selected.label.clone()
                } else {
                    format!("{}\nNote: {}", selected.label, input)
                }
            },
            None => input
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| question.choices.get(index))
                .map_or(input.clone(), |choice| choice.label.clone()),
        };
        self.interview_drafts[usize::from(!first)] = Some((choice, input));
        let receipt = self.next_interview_activity(request.activity().turn())?;
        self.event(AgentEvent::ActivityStarted {
            activity: receipt,
            kind: ActivityKind::UserInputResponse {
                request_id: request.request_id(),
            },
        });
        self.event(AgentEvent::ActivityUpdated {
            activity: receipt,
            update: ActivityUpdate::TextSnapshot(format!(
                "Question {} of 2\nAnswer: {resolved}\n\nOffline preview · {}",
                if first { 1 } else { 2 },
                if first {
                    "one question remains."
                } else {
                    "both answers recorded."
                }
            )),
        });
        self.event(AgentEvent::ActivityFinished {
            activity: receipt,
            outcome: ActivityOutcome::Completed,
        });
        if first {
            self.event(AgentEvent::ActivityFinished {
                activity: request.activity(),
                outcome: ActivityOutcome::Completed,
            });
            self.show_interview(request.activity().turn(), false, Some(resolved))?;
        } else {
            self.finish_answer(request, format!("## Preferences recorded\n\nPriority: {}\n\nDensity: {resolved}\n\nBoth answers stayed in this offline preview.", first_answer.expect("first question answered")));
        }
        Ok(())
    }

    fn finish_answer(&mut self, request: ActivityRequestRef, text: String) {
        self.request = None;
        self.active = None;
        self.event(AgentEvent::ActivityFinished {
            activity: request.activity(),
            outcome: ActivityOutcome::Completed,
        });
        let activity = ActivityRef::new(
            request.activity().turn(),
            ActivityId::new(NonZeroU64::new(10).unwrap()),
        );
        self.event(AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        });
        self.event(AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(text),
        });
        self.event(AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        });
        self.event(AgentEvent::TurnFinished {
            turn: activity.turn(),
            outcome: TurnOutcome::Completed,
        });
    }

    fn interrupt(&mut self) {
        let interrupted_request = self.request.take();
        if let Some(activity) = self.active.take() {
            self.stream.clear();
            self.event(AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Interrupted,
            });
            if let Some(PreviewRequest::Interview { first_answer, .. }) = interrupted_request {
                let recorded = usize::from(first_answer.is_some());
                let notice = ActivityNotice {
                    level: NoticeLevel::Warning,
                    title: "Interview incomplete".to_owned(),
                    message: format!("{recorded}/2 answers recorded · {} unanswered\nOffline preview · submission incomplete.\n1. {}: What should the chat make easiest?\n2. Unanswered: How much detail should stay visible?", 2 - recorded, if recorded == 1 { "Recorded" } else { "Unanswered" }),
                }.to_snapshot().expect("bounded offline summary");
                let summary = ActivityRef::new(
                    activity.turn(),
                    ActivityId::new(NonZeroU64::new(13).unwrap()),
                );
                self.event(AgentEvent::ActivityStarted {
                    activity: summary,
                    kind: ActivityKind::ModelWork,
                });
                self.event(AgentEvent::ActivityUpdated {
                    activity: summary,
                    update: ActivityUpdate::TextSnapshot(notice),
                });
                self.event(AgentEvent::ActivityFinished {
                    activity: summary,
                    outcome: ActivityOutcome::Completed,
                });
            }
            self.event(AgentEvent::TurnFinished {
                turn: activity.turn(),
                outcome: TurnOutcome::Interrupted,
            });
        }
    }
}

impl AgentConnection for TestAgent {
    type Error = io::Error;

    fn dispatch(&mut self, action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        match action {
            AgentAction::Submit(submission) | AgentAction::Steer { submission, .. } => {
                if self.active.is_some() {
                    return Ok(DispatchOutcome::Rejected { id: submission.id(), rejection: SubmissionRejection::new(
                        SubmissionRejectionKind::Incompatible, "Test agent is busy. Interrupt it, then send again.") });
                }
                let input = submission.input().as_str().to_owned();
                self.ready.push_back(AgentPoll::Submission(SubmissionOutcome::Accepted { id: submission.id() }));
                if input.trim() == "session-document" {
                    let document = ActivityDocument { title: "Workspace guide".into(), markdown: "This **host document** did not start a model turn.\n\n| Action | Shortcut |\n| --- | --- |\n| Inspect output | `/output` |\n| Review changes | `/changes` |\n\n```rust\nuse std::path::Path;\n```".into() };
                    self.ready.push_back(AgentPoll::Document(TuiDocument::new(document).expect("bounded session document").with_expanded(true)));
                    return Ok(DispatchOutcome::Queued);
                }
                if matches!(input.trim(), "status" | "status-update" | "status-clear") {
                    let status = match input.trim() {
                        "status" => TuiStatusLine::new([
                            ("branch", "Demo branch: chat-ui"),
                            ("checks", "Demo checks: 18 passed"),
                            ("worker", "Demo worker: idle"),
                        ]).expect("bounded offline status"),
                        "status-update" => TuiStatusLine::new([
                            ("checks", "Demo checks: 20 passed"),
                            ("worker", "Demo worker: ready"),
                        ]).expect("bounded offline status"),
                        _ => TuiStatusLine::default(),
                    };
                    self.ready.push_back(AgentPoll::StatusLine(status));
                    return Ok(DispatchOutcome::Queued);
                }
                let notice = match input.trim() {
                    "warning" => Some(("Codex configuration warning", "Preview only: an unsupported setting was ignored.\nFile: config.toml\nThis session notice does not start a model turn.")),
                    "deprecation" => Some(("Codex deprecation notice", "Preview only: a legacy setting will be removed.\nDetails: migrate to the replacement setting before upgrading.\nFile: config.toml")),
                    "approval-warning" => Some(("Codex approval warning", "Preview only: automatic approval review is unavailable.\nRead the request scope before choosing an action.\nNo command was executed and no permission was changed.")),
                    _ => None,
                };
                if let Some((title, message)) = notice {
                    self.ready.push_back(AgentPoll::Notice(ActivityNotice {
                        title: title.to_owned(),
                        message: message.to_owned(),
                        level: NoticeLevel::Warning,
                    }));
                    return Ok(DispatchOutcome::Queued);
                }
                self.turns += 1;
                let turn = TurnRef::new(self.session, TurnId::new(NonZeroU64::new(self.turns).unwrap()));
                let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::MIN));
                self.ready.push_back(AgentPoll::Record(TranscriptRecord::CommandCommitted(
                    AgentCommand::StartTurn { turn, input: submission.into_input() })));
                self.event(AgentEvent::TurnStarted { turn });
                if input.trim() == "proposed-plan" {
                    self.active=Some(activity);
                    self.event(AgentEvent::ActivityStarted{activity,kind:ActivityKind::ModelWork});
                    self.event(AgentEvent::ActivityUpdated{activity,update:ActivityUpdate::TextSnapshot(ActivityDocument{title:"Proposed plan".to_owned(),markdown:"## Draft\n\nInspecting the current implementation…".to_owned()}.to_snapshot().expect("bounded preview document"))});
                    self.stream.push_back(AgentEvent::ActivityUpdated{activity,update:ActivityUpdate::TextSnapshot(response("proposed-plan").1)});
                    self.stream.push_back(AgentEvent::ActivityFinished{activity,outcome:ActivityOutcome::Completed});
                    self.stream.push_back(AgentEvent::TurnFinished{turn,outcome:TurnOutcome::Completed});
                    self.next=Instant::now()+Duration::from_secs(1);
                    self.armed=false;
                    return Ok(DispatchOutcome::Queued);
                }
                if input.trim() == "shell-progress" {
                    self.active = Some(activity);
                    self.event(AgentEvent::ActivityStarted {activity,kind:ActivityKind::ToolCall});
                    let output = ToolOutput {tool:"run_command".to_owned(),server:Some("preview".to_owned()),arguments:Some(json!({"command":"cargo test --workspace"})),result:Some(json!({"content":[{"type":"text","text":json!({"stdout":"First output while the command is running.\nOffline fixture: no command was executed.","stderr":""}).to_string()}],"progress":true})),content_items:None,error:None,plain_text:"Offline command progress fixture".to_owned()};
                    self.event(AgentEvent::ActivityUpdated {activity,update:ActivityUpdate::TextSnapshot(output.to_snapshot().expect("bounded progress fixture"))});
                    self.stream.push_back(AgentEvent::ActivityUpdated {activity,update:ActivityUpdate::TextSnapshot(response("shell").1)});
                    self.stream.push_back(AgentEvent::ActivityFinished {activity,outcome:ActivityOutcome::Completed});
                    self.stream.push_back(AgentEvent::TurnFinished {turn,outcome:TurnOutcome::Completed});
                    self.next=Instant::now()+Duration::from_secs(1);
                    self.armed=false;
                    return Ok(DispatchOutcome::Queued);
                }
                if input.trim() == "compaction" {
                    self.active = Some(activity);
                    self.event(AgentEvent::ActivityStarted { activity, kind: ActivityKind::ModelWork });
                    self.event(AgentEvent::ActivityUpdated { activity, update: ActivityUpdate::TextSnapshot(ActivityNotice {
                        title: "Compacting context".to_owned(),
                        message: "Preview only: context compaction in progress.".to_owned(),
                        level: NoticeLevel::Info,
                    }.to_snapshot().expect("bounded preview notice")) });
                    self.stream.push_back(AgentEvent::ActivityUpdated { activity, update: ActivityUpdate::TextSnapshot(response("compaction").1) });
                    self.stream.push_back(AgentEvent::ActivityFinished { activity, outcome: ActivityOutcome::Completed });
                    self.stream.push_back(AgentEvent::TurnFinished { turn, outcome: TurnOutcome::Completed });
                    self.next = Instant::now() + Duration::from_secs(1);
                    self.armed = false;
                    return Ok(DispatchOutcome::Queued);
                }
                if matches!(input.trim(), "approval" | "approval-scopes" | "approval-diff") {
                    let request_id = RequestId::new(NonZeroU64::MIN);
                    let kind = match input.trim() {
                        "approval-scopes" => PreviewApproval::Scopes,
                        "approval-diff" => PreviewApproval::FileChange,
                        _ => PreviewApproval::Plain,
                    };
                    if kind == PreviewApproval::FileChange {
                        for (id, path) in [(2, "src/proposed.rs"), (3, "src/unrelated.rs")] {
                            let change = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(id).unwrap()));
                            self.event(AgentEvent::ActivityStarted { activity:change, kind:ActivityKind::FileChange });
                            self.event(AgentEvent::ActivityUpdated { activity:change, update:ActivityUpdate::TextSnapshot(format!("update: {path}\n@@ -1 +1 @@\n-let ready = false;\n+let ready = true;")) });
                            self.event(AgentEvent::ActivityFinished { activity:change, outcome:ActivityOutcome::Completed });
                        }
                    }
                    self.request = Some(PreviewRequest::Approval(ActivityRequestRef::new(activity, request_id), kind));
                    if let Some(profile) = kind.profile() {
                        self.ask(activity, ActivityKind::ApprovalRequest { request_id }, &profile.to_snapshot().expect("bounded preview approval"));
                        return Ok(DispatchOutcome::Queued);
                    }
                    self.ask(activity, ActivityKind::ApprovalRequest { request_id },
                        "Run the layout checks?\n\nCommand: cargo test --locked -p yo-tui\nWorking directory: /workspace/yo\nAdditional permissions: {\"network\":{\"enabled\":false}}\nScope: requested command\nReason: verify rendering and keyboard interactions.\n\nPreview only: no command will run.");
                    return Ok(DispatchOutcome::Queued);
                }
                if input.trim() == "interview" {
                    let request_id = RequestId::new(NonZeroU64::MIN);
                    self.request = Some(PreviewRequest::Interview { request: ActivityRequestRef::new(activity, request_id), first_answer: None, first: true });
                    self.interview_drafts = [None, None];
                    self.interview_sequence = 100;
                    self.ask(activity, ActivityKind::UserInputRequest { request_id },
                        &interview_prompt(true));
                    return Ok(DispatchOutcome::Queued);
                }
                if let Some((kind, response)) = media_response(input.trim()).map(|text| (ActivityKind::AgentMessage, text))
                    .or_else(|| matches!(input.trim(), "agent-tasks" | "file-links" | "usage" | "plan" | "reasoning" | "agent-reasoning" | "tool-diff" | "mcp-image" | "retry" | "reroute" | "turn-diff" | "summary" | "branch" | "terminal-wait" | "terminal-input" | "turn-duration" | "file-read" | "file-write" | "files-read" | "file-edit" | "shell" | "codex-shell" | "shell-tail" | "shell-truncated" | "shell-retained" | "files-list" | "files-find" | "content-search" | "resource-link" | "embedded-resource").then(|| response(input.trim()))) {
                    self.event(AgentEvent::ActivityStarted { activity, kind });
                    self.event(AgentEvent::ActivityUpdated { activity, update: ActivityUpdate::TextSnapshot(response) });
                    self.event(AgentEvent::ActivityFinished { activity, outcome: ActivityOutcome::Completed });
                    self.event(AgentEvent::TurnFinished { turn, outcome: TurnOutcome::Completed });
                    return Ok(DispatchOutcome::Queued);
                }
                let (kind, response) = response(&input);
                self.event(AgentEvent::ActivityStarted { activity, kind });
                self.active = Some(activity);
                for chunk in response.split_inclusive(' ') {
                    self.stream.push_back(AgentEvent::ActivityUpdated { activity,
                        update: ActivityUpdate::TextDelta(chunk.to_owned()) });
                }
                let failed = matches!(input.trim(), "error" | "mcp-failure");
                self.stream.push_back(AgentEvent::ActivityFinished { activity, outcome: if failed {
                    ActivityOutcome::Failed(Failure::new("Simulated tool failure; no command was executed."))
                } else { ActivityOutcome::Completed } });
                self.stream.push_back(AgentEvent::TurnFinished { turn, outcome: if failed {
                    TurnOutcome::Failed(Failure::new("Preview failure. Send another message to recover."))
                } else { TurnOutcome::Completed } });
                self.next = Instant::now();
            },
            AgentAction::RespondToApproval { request, decision } => {
                if let Some(PreviewRequest::Approval(current, kind)) = self.request
                    && current == request
                {
                    let (decision_text, declined) = match decision {
                        ApprovalDecision::Approved if kind == PreviewApproval::Plain => ("approved".to_owned(), false),
                        ApprovalDecision::Declined if kind == PreviewApproval::Plain => ("declined".to_owned(), true),
                        ApprovalDecision::Offered(choice) if kind != PreviewApproval::Plain => {
                            let profile = kind.profile().expect("structured preview approval");
                            let selected = choice.checked_sub(1).and_then(|index| profile.choices.get(index as usize))
                                .filter(|choice| choice.enabled).ok_or_else(|| io::Error::other("unsupported preview approval choice"))?;
                            (format!("{}\n{}", selected.label, selected.description), profile.decline_choice == Some(choice))
                        },
                        _ => return Err(io::Error::other("approval response does not match preview choices")),
                    };
                    let receipt = ActivityRef::new(request.activity().turn(), ActivityId::new(NonZeroU64::new(14).unwrap()));
                    self.event(AgentEvent::ActivityStarted { activity: receipt, kind: ActivityKind::ApprovalResponse { request_id: request.request_id() } });
                    self.event(AgentEvent::ActivityUpdated { activity: receipt, update: ActivityUpdate::TextSnapshot(format!("Decision: {decision_text}")) });
                    self.event(AgentEvent::ActivityFinished { activity: receipt, outcome: ActivityOutcome::Completed });
                    let text = if declined {
                        "## Declined\n\nThe action was not run. You can continue the conversation."
                    } else {
                        "## Approval recorded\n\nThe decision stayed in this offline preview. No command was executed. No permission or policy was changed."
                    };
                    self.finish_answer(request, text.to_owned());
                }
            },
            AgentAction::PreviousQuestion { request, choice, draft } => {
                let Some(PreviewRequest::Interview { request: current, first: false, first_answer }) = &self.request else {
                    return Err(io::Error::other("previous preview question is unavailable"));
                };
                if *current != request || choice.is_some_and(|choice| choice == 0 || choice > 2) {
                    return Err(io::Error::other("previous preview question or choice is stale"));
                }
                let first_answer = first_answer.clone();
                self.interview_drafts[1] = Some((choice, draft));
                self.event(AgentEvent::ActivityFinished { activity: request.activity(), outcome: ActivityOutcome::Completed });
                self.show_interview(request.activity().turn(), true, first_answer)?;
            },
            AgentAction::RespondToQuestion { request, choice, notes } => {
                self.answer_interview(request, notes, Some(choice))?;
            },
            AgentAction::RespondToUserInput { request, input } => {
                self.answer_interview(request, input, None)?;
            },
            AgentAction::Interrupt => self.interrupt(),
            _ => self.ready.push_back(AgentPoll::Control(AgentControlOutcome::ContextCompactionRejected {
                detail: "This offline test agent supports chat, tools, long-tools, error, long, markdown, tables, diff, approval, interview, and interruption only.".to_owned() })),
        }
        Ok(DispatchOutcome::Queued)
    }

    fn retry(&mut self, _: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Err(io::Error::other(
            "preview never creates backpressured commands",
        ))
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        if let Some(record) = self.ready.pop_front() {
            return Ok(record);
        }
        if Instant::now() >= self.next
            && let Some(event) = self.stream.pop_front()
        {
            if matches!(event, AgentEvent::TurnFinished { .. }) {
                self.active = None;
            }
            self.next = Instant::now() + Duration::from_millis(70);
            self.armed = false;
            return Ok(AgentPoll::Record(TranscriptRecord::EventCommitted(event)));
        }
        Ok(AgentPoll::Pending)
    }

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        if !self.ready.is_empty() || (!self.stream.is_empty() && Instant::now() >= self.next) {
            return Poll::Ready(());
        }
        if !self.stream.is_empty() && !self.armed {
            self.armed = true;
            let delay = self.next.saturating_duration_since(Instant::now());
            let waker = cx.waker().clone();
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                waker.wake();
            });
        }
        Poll::Pending
    }
}

fn approval_profile() -> ActivityApproval {
    ActivityApproval {
        related_change: None,
        plain_text: "Command: cargo test\nWorking directory: /workspace/yo\nOffline preview: no command, permission or policy change.\n\nSession scope: future matching prompts in this session.\nPersistent command rule: prefix [\"cargo\",\"test\"].\nPersistent network rules: host example.com.".to_owned(),
        choices: [
            ("Approve request", "Use the scope described above"),
            ("Approve for session", "Future matching prompts in this session may run without asking again"),
            ("Approve + save rule", "Persistent rule for commands starting with cargo test"),
            ("Always allow host", "Persistent network allow rule for example.com"),
            ("Always deny host", "Persistent network deny rule for example.com"),
            ("Decline", "Do not run this action; continue the turn"),
        ].into_iter().map(|(label, description)| ApprovalChoice { label:label.to_owned(), description:description.to_owned(), enabled:true }).collect(),
        decline_choice: Some(6),
    }
}

fn interview_prompt(first: bool) -> String {
    let (plain_text, choices) = if first {
        (
            "Question 1 of 2 · Priority\n\nWhat should the chat make easiest?\n1. Reading code and explanations\n2. Reviewing changes and approvals\n3. None of the above\n\nChoose an option or describe your preference.",
            [
                ("Reading code and explanations", "Readable code and answers"),
                ("Reviewing changes and approvals", "Inspect proposed edits"),
            ],
        )
    } else {
        (
            "Question 2 of 2 · Density\n\nHow much detail should stay visible?\n1. Compact summaries, expand when needed\n2. More detail in the conversation\n\nEnter 1, 2, or write your own answer.",
            [
                (
                    "Compact summaries, expand when needed",
                    "Keep the conversation short",
                ),
                (
                    "More detail in the conversation",
                    "Show more output by default",
                ),
            ],
        )
    };
    let mut choices = choices
        .into_iter()
        .map(|(label, description)| QuestionChoice {
            label: label.to_owned(),
            description: description.to_owned(),
        })
        .collect::<Vec<_>>();
    if first {
        choices.push(QuestionChoice {
            label: "None of the above".to_owned(),
            description: "Choose an answer outside this list.".to_owned(),
        });
    }
    ActivityQuestion {
        allow_notes: true,
        previous_question: false,
        draft: None,
        draft_choice: None,
        plain_text: plain_text.to_owned(),
        choices,
    }
    .to_snapshot()
    .expect("bounded interview fixture")
}

fn response(input: &str) -> (ActivityKind, String) {
    match input.trim() {
        "tools" => (ActivityKind::ToolCall, "[SIMULATED] cargo test\nChecking layout and Unicode wrapping...\nAll preview checks passed. No process or filesystem operation was performed.".into()),
        "long-tools" => (ActivityKind::ToolCall, (1..=30).map(|n| format!("[SIMULATED] Check {n:02}: passed.\n")).collect()),
        "error" => (ActivityKind::ToolCall, "[SIMULATED] Checking a missing fixture...\nThis request deliberately fails so you can inspect error presentation.".into()),
        "tables" => (ActivityKind::AgentMessage, "| Component | Status | Tests |\n| :--- | :---: | ---: |\n| Rendering | **ready** | 32 |\n| 한글 wrapping | ready | 12 |\n| Theme | `mono` | 8 |".into()),
        "file-links" => (ActivityKind::AgentMessage, "## Host-confirmed file link\n\n[README.md](README.md)\n\nREADME.md points to a file on the yo execution host. Opening it requires a compatible terminal file handler.\n\n[Unmapped file](missing.rs) and [raw file URL](file:///tmp/untrusted) stay plain.\n\n| Kind | Link |\n| --- | --- |\n| Registered file | [README.md](README.md) |\n| Web | [Documentation](https://example.com/docs) |\n\n```text\n[README.md](README.md)\n```\n\nOver SSH, this file URL does not download or open the remote file inside yo. Web links are independent. This preview does not open files.".into()),
        "usage" => (ActivityKind::ModelWork, r#"{"schema":"codex.app-server-token-usage-receipt/v1","source_profile":"codex.app-server.thread-token-usage-updated/v1","turn_id":"preview-only","model_context_window":200000,"usage":{"input_tokens":12500,"output_tokens":1800,"total_tokens":14300,"reasoning_tokens":600,"cache_read_input_tokens":9800,"cache_write_input_tokens":0}}"#.into()),
        "agent-tasks" => (ActivityKind::ToolCall, ToolOutput {
            tool: "wait".to_owned(), server: None, arguments: None, result: None, error: None,
            content_items: Some(json!([{"type":"text","text":"Agent task · wait\nTool status: completed\nReported agent states:\nAgent reviewer · completed\nReview finished.\n\nAgent tests · running\nChecks are still running.\n\nAgent docs · errored\nSource document unavailable.\n\nOffline example: no agents were started.","source":{"tool":"wait","status":"completed","agentsStates":{"reviewer":{"status":"completed","message":"Review finished."},"tests":{"status":"running","message":"Checks are still running."},"docs":{"status":"errored","message":"Source document unavailable."}}}}])),
            plain_text: "Agent task · wait\nTool status: completed\nAgent reviewer · completed\nAgent tests · running\nAgent docs · errored\nOffline example: no agents were started.".to_owned(),
        }.to_snapshot().expect("bounded agent task fixture")),
        "mcp" | "mcp-failure" => (ActivityKind::ToolCall, "docs.search\nArguments:\n  query: transcript wrapping\nResult:\nFound 3 matching pages.\nPreview only: no external tool was called.\n\nResource · Rendering guide\nURI: memory://docs/rendering\nNarrow output wraps within the configured body width.\n\nstructuredContent:\n{\"matches\": 3}".into()),
        "embedded-resource" => (ActivityKind::ToolCall, ToolOutput {
            tool:"lookup".to_owned(),server:Some("offline".to_owned()),arguments:None,
            result:Some(json!({"content":[{"type":"resource","resource":{"uri":"resource://preview/main.rs","mimeType":"text/x-rust","text":"use std::path::Path;\n\nfn main() {\n    let path = Path::new(\"demo\");\n}","_meta":{"revision":7}},"annotations":{"audience":["user"]}},{"type":"resource","resource":{"uri":"resource://preview/image","mimeType":"image/png","blob":sample_image_data()}}]})),
            content_items:None,error:None,plain_text:"Offline embedded resources: Rust source, metadata and PNG bytes. Nothing was fetched.".to_owned(),
        }.to_snapshot().expect("bounded embedded resource fixture")),
        "resource-link" => (ActivityKind::ToolCall, ToolOutput {
            tool:"resources".to_owned(), server:Some("offline".to_owned()), arguments:None,
            result:Some(json!({"content":[{"type":"resource_link","name":"build-report.txt","title":"Build report","uri":"resource://preview/build-report","mimeType":"text/plain","size":2048,"description":"Offline resource reference. Nothing was fetched.","annotations":{"audience":["user"]}}]})),
            content_items:None,error:None,plain_text:"Offline resource link: build-report.txt, resource://preview/build-report, text/plain, 2048 bytes. Nothing was fetched.".to_owned(),
        }.to_snapshot().expect("bounded resource link fixture")),
        "content-search" => (ActivityKind::ToolCall, ToolOutput {
            tool: "grep".to_owned(), server: Some("offline".to_owned()),
            arguments: Some(json!({"pattern":"render","path":"src","glob":"*.rs"})),
            result: Some(json!({"content":[{"type":"text","text":"view.rs:12: fn render() {\nview.rs-13-     // surrounding context"}],"details":{"matchLimitReached":1,"linesTruncated":true}})),
            content_items: None, error: None,
            plain_text: "Offline content search: no search was executed.\nview.rs:12: fn render() {\nview.rs-13-     // surrounding context\nMatch limit reached: 1; some lines truncated.".to_owned(),
        }.to_snapshot().expect("bounded content search fixture")),
        "files-find" => (ActivityKind::ToolCall, ToolOutput {
            tool: "find".to_owned(), server: Some("offline".to_owned()),
            arguments: Some(json!({"pattern":"**/*.rs","path":"src","limit":2})),
            result: Some(json!({"content":[{"type":"text","text":"src/main.rs\nsrc/lib.rs"}],"details":{"resultLimitReached":2}})),
            content_items: None, error: None,
            plain_text: "Offline file search: no filesystem search was executed.\nsrc/main.rs\nsrc/lib.rs\nResult limit reached: 2".to_owned(),
        }.to_snapshot().expect("bounded file search fixture")),
        "files-list" => (ActivityKind::ToolCall, ToolOutput {
            tool:"list_files".to_owned(), server:Some("preview".to_owned()),
            arguments:Some(json!({"path":"."})),
            result:Some(json!({"content":[{"type":"text","text":"Cargo.toml\nREADME.md\nsrc/\ntests/\n\n[yo: tool output truncated]"}],"truncated":true,"note":"Offline fixture: no directory was read."})),
            content_items:None, error:None,
            plain_text:"preview.list_files\nCargo.toml\nREADME.md\nsrc/\ntests/\n\n[yo: tool output truncated]\nOffline fixture: no directory was read.".to_owned(),
        }.to_snapshot().expect("bounded directory fixture")),
        "shell-retained" => {
            let retained = (1..=120).map(|row| format!("Captured row {row:03}: original command output")).collect::<Vec<_>>().join("\n");
            let preview = "status: 0\nstdout:\nCaptured row 001\n[yo: bytes omitted from model result]\nCaptured row 120\nstderr:\n";
            (ActivityKind::ToolCall, ToolOutput {
                tool: "run_command".to_owned(), server: None,
                arguments: Some(json!({"command":"offline retained-output example"})),
                result: Some(json!({"content":[{"type":"text","text":preview}],"truncated":true,"retainedOutput":{"truncated":false}})),
                content_items: None, error: None,
                plain_text: format!("Offline fixture: no command was executed.\n{retained}"),
            }.to_snapshot().expect("bounded retained output fixture"))
        },
        "shell-truncated" => {
            let text = "Last reported output line.\nOffline fixture: no output file was created.";
            (ActivityKind::ToolCall, ToolOutput {
                tool:"bash".to_owned(),server:None,
                arguments:Some(json!({"command":"cargo test --workspace"})),
                result:Some(json!({"content":[{"type":"text","text":text}],"truncated":true,"details":{"fullOutputPath":"offline-preview/build-output.log","truncation":{"content":text,"truncated":true,"truncatedBy":"lines","totalLines":100,"totalBytes":9000,"outputLines":2,"outputBytes":text.len(),"lastLinePartial":false,"firstLineExceedsLimit":false,"maxLines":2,"maxBytes":51200}}})),
                content_items:None,error:None,plain_text:format!("{text}\nReported full output file: offline-preview/build-output.log"),
            }.to_snapshot().expect("bounded truncation fixture"))
        },
        "shell-tail" => {
            let text = format!("{}\nLatest result: preview checks passed.\nOffline fixture: no command was executed.", (1..=20).map(|n|format!("test preview_case_{n:02} ... ok")).collect::<Vec<_>>().join("\n"));
            (ActivityKind::ToolCall, ToolOutput {
                tool:"commandExecution".to_owned(),server:None,
                arguments:Some(json!({"command":"cargo test --workspace","cwd":"/workspace/yo"})),
                result:Some(json!({"content":[{"type":"text","text":text}],"status":"completed","exitCode":0,"durationMs":1240})),
                content_items:None,error:None,plain_text:text,
            }.to_snapshot().expect("bounded shell tail fixture"))
        },
        "codex-shell" => (ActivityKind::ToolCall, ToolOutput {
            tool:"commandExecution".to_owned(), server:None,
            arguments:Some(json!({"command":"if true; then\n  cargo test --workspace\nfi","cwd":"/workspace/yo"})),
            result:Some(json!({"content":[{"type":"text","text":"Offline fixture: no command was executed.\nCombined process output stays literal."}],"status":"completed","exitCode":0,"durationMs":1240})),
            content_items:None, error:None,
            plain_text:"cargo test --workspace\nOffline fixture: no command was executed.\nExit: 0 · Duration: 1240 ms".to_owned(),
        }.to_snapshot().expect("bounded Codex command fixture")),
        "shell" => (ActivityKind::ToolCall, ToolOutput {
            tool:"run_command".to_owned(), server:Some("preview".to_owned()),
            arguments:Some(json!({"command":"cargo test --workspace"})),
            result:Some(json!({"content":[{"type":"text","text":"status: 0\nstdout:\nOffline fixture: no command was executed.\nAll preview checks passed.\nstderr:\n"}]})),
            content_items:None, error:None,
            plain_text:"preview.run_command\ncargo test --workspace\nstatus: 0\nstdout:\nOffline fixture: no command was executed.\nAll preview checks passed.\nstderr:\n".to_owned(),
        }.to_snapshot().expect("bounded shell fixture")),
        "file-edit" => (ActivityKind::ToolCall, ToolOutput {
            tool:"edit_file".to_owned(), server:Some("preview".to_owned()),
            arguments:Some(json!({"path":"src/greeting.rs","edits":[
                {"oldText":"    \"Hello\"\n","newText":"    \"Hello from Yo\"\n"},
                {"oldText":"// obsolete note","newText":""}
            ]})),
            result:Some(json!({"content":[{"type":"text","text":"Offline fixture: no replacements were applied."}]})),
            content_items:None, error:None,
            plain_text:"preview.edit_file\nsrc/greeting.rs\nProposed replacements: Hello -> Hello from Yo; remove obsolete note.\nOffline fixture: no replacements were applied.".to_owned(),
        }.to_snapshot().expect("bounded edit fixture")),
        "files-read" => (ActivityKind::ToolCall, sample_batch_read()),
        "file-write" => (ActivityKind::ToolCall, ToolOutput {
            tool:"write_file".to_owned(), server:Some("preview".to_owned()),
            arguments:Some(r#"{"path":"src/greeting.rs","content":"pub fn greeting() -> &'static str {\n    \"Hello from Yo\"\n}"}"#.parse().expect("fixture write arguments")),
            result:Some(r#"{"content":[{"type":"text","text":"Offline fixture: no file was written."}]}"#.parse().expect("fixture write result")),
            content_items:None, error:None,
            plain_text:"preview.write_file\nsrc/greeting.rs\npub fn greeting() -> &'static str {\n    \"Hello from Yo\"\n}\nOffline fixture: no file was written.".to_owned(),
        }.to_snapshot().expect("bounded write fixture")),
        "file-read" => (ActivityKind::ToolCall, ToolOutput {
            tool:"read".to_owned(), server:Some("preview".to_owned()),
            arguments:Some(r#"{"path":"src/main.rs","offset":1,"limit":5}"#.parse().expect("fixture arguments")),
            result:Some(r#"{"content":[{"type":"text","text":"use std::fmt;\n\nfn main() {\n    println!(\"offline file preview\");\n}"}],"note":"Offline fixture: no file was read."}"#.parse().expect("fixture result")),
            content_items:None, error:None,
            plain_text:"preview.read\nsrc/main.rs\nuse std::fmt;\n\nfn main() {\n    println!(\"offline file preview\");\n}\nOffline fixture: no file was read.".to_owned(),
        }.to_snapshot().expect("bounded file fixture")),
        "tool-diff" => {
            let patch = "--- settings.rs\n+++ settings.rs\n@@ -1,3 +1,3 @@\n fn settings() {\n-    let theme = \"fixed\";\n+    let theme = \"selected\";\n }\n";
            (ActivityKind::ToolResult, ToolOutput {
                tool: "workspace_patch".to_owned(), server: None, arguments: None,
                result: None, error: None,
                content_items: Some(json!([{
                    "type":"diff", "title":"File change · settings.rs", "text":patch,
                    "source":{"path":"settings.rs", "oldText":"fn settings() {\n    let theme = \"fixed\";\n}\n", "newText":"fn settings() {\n    let theme = \"selected\";\n}\n"}
                }])),
                plain_text: format!("Offline fixture: no file was changed.\n{patch}"),
            }.to_snapshot().expect("bounded generic diff fixture"))
        },
        "mcp-image" => (ActivityKind::ToolCall, sample_tool_image()),
        "summary" | "branch" => (ActivityKind::ModelWork, ActivitySummary {
            kind: if input == "branch" { SummaryKind::Branch } else { SummaryKind::Compaction },
            tokens_before: (input == "summary").then_some(24000),
            summary: "## Offline summary example\n\nThis is fixture content, not a summary of your session.\n\n### Decisions\n\n- Keep source text in the journal.\n- Wrap code and tables to the available width.\n- Preserve approval ownership.\n- Show actual tool outcomes.\n- Keep images behind explicit media output.\n- Apply the selected palette.\n- Retain the complete summary when collapsed.\n- Keep interruption details visible.\n\n### Next step\n\n```rust\nuse std::fmt;\n```\n\nReview the implementation and run the affected checks.".to_owned(),
        }.to_snapshot().expect("bounded preview summary")),
        "compaction" => (ActivityKind::ModelWork, ActivityNotice { title:"Context compacted".to_owned(), message:"Preview only: conversation context compacted.\nNo summary or token counts were reported.".to_owned(), level:NoticeLevel::Info }.to_snapshot().expect("bounded preview notice")),
        "turn-diff" => (ActivityKind::FileChange, "Turn aggregate diff\ndiff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-fn main() {}\n+fn main() { println!(\"ready\"); }\ndiff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-Old instructions\n+Updated instructions\n".to_owned()),
        "reroute" => (ActivityKind::ModelWork, ActivityNotice {title:"Model rerouted".to_owned(), message:"Preview only: provider-reported model change.\nFrom: requested-model\nTo: reported-model\nReason: highRiskCyberActivity\nNo provider call or model selection change occurred.".to_owned(),level:NoticeLevel::Warning}.to_snapshot().expect("bounded preview notice")),
        "retry" => (ActivityKind::ModelWork, ActivityNotice {title:"Retry announced".to_owned(), message:"Preview only: temporary connection interruption.\nThe provider reported that it will retry.".to_owned(),level:NoticeLevel::Warning}.to_snapshot().expect("bounded preview notice")),
        "diagrams" => (ActivityKind::AgentMessage, "## Diagrams\n\n```mermaid\ngraph LR; A[Request] --> B[Review] --> C[Apply]\n```\n\n```mermaid\nsequenceDiagram\nUser->>Agent: Inspect changes\nAgent-->>User: Review result\n```\n\nNarrow views keep the Mermaid source instead of clipping connections.".to_owned()),
        "agent-reasoning" => (ActivityKind::ModelWork, ActivityReasoning { content: json!("Offline reasoning fixture.\n\nChecking code, table and diff rendering at narrow widths.") }.to_snapshot().expect("bounded reasoning fixture")),
        "reasoning" => (ActivityKind::ModelWork, ActivitySummary {kind: SummaryKind::Reasoning, summary:"Inspecting the output components\n\nChecking narrow widths and configurable folding.".to_owned(), tokens_before:None}.to_snapshot().expect("bounded preview summary")),
        "search" => (ActivityKind::ToolCall, ToolOutput {
            tool: "webSearch".to_owned(), server: None,
            arguments: Some(json!({"query":"terminal output components", "action":{"type":"search", "queries":["terminal output components", "configurable rendering"]}})),
            result: None, content_items: None, error: None,
            plain_text: "Web search\nQuery: terminal output components\nQuery: configurable rendering".to_owned(),
        }.to_snapshot().expect("bounded search preview")),
        "proposed-plan" => (ActivityKind::ModelWork,ActivityDocument{title:"Proposed plan".to_owned(),markdown:"## Implementation approach\n\nOffline fixture: this document does not start a task or approve an action.\n\n1. Inspect the existing renderer.\n2. Preserve authoritative source through streaming.\n3. Reflow tables and code after terminal resize.\n4. Validate empty and interrupted states.\n\n```rust\nuse std::fmt;\n```\n\n| Check | Expected |\n| --- | --- |\n| Resize | Reflow |\n| Source | Preserved |".to_owned()}.to_snapshot().expect("bounded preview document")),
        "turn-duration" => (ActivityKind::ModelWork, ActivityNotice { title:"Turn completed".to_owned(), message:"Duration: 1m 2.345s (62345 ms, reported by Codex)\nOffline fixture: illustrative server duration.".to_owned(), level:NoticeLevel::Info }.to_snapshot().expect("bounded duration preview")),
        "terminal-wait" => (ActivityKind::ModelWork,ActivityDocument{title:"Waited for background terminal".to_owned(),markdown:"```text\nProcess: preview-7\nCommand: cargo test\n```\n\nOffline fixture: no process was polled.".to_owned()}.to_snapshot().expect("bounded terminal preview")),
        "terminal-input" => (ActivityKind::ModelWork,ActivityDocument{title:"Terminal input sent".to_owned(),markdown:"````text\nProcess: preview-7\nCommand: cargo test\nInput:\n```\nThis is literal terminal input.\n````\n\nOffline fixture: no input was sent.".to_owned()}.to_snapshot().expect("bounded terminal preview")),
        "plan" => (ActivityKind::ModelWork, ActivityPlan {
            explanation:Some("Offline example: observed task progress.".to_owned()),
            steps: vec![
                PlanStep {text:"Inspect the renderer".to_owned(),status:PlanStepStatus::Completed},
                PlanStep {text:"Connect real backend events".to_owned(),status:PlanStepStatus::Completed},
                PlanStep {text:"Verify narrow-terminal navigation and preserve wrapped task descriptions".to_owned(),status:PlanStepStatus::InProgress},
                PlanStep {text:"Review the final output".to_owned(),status:PlanStepStatus::Pending},
            ],
        }.to_snapshot().expect("bounded preview plan")),
        "changes" => (ActivityKind::FileChange, "update: src/settings.rs\n@@ -1,3 +1,5 @@\n fn settings() {\n-    let theme = \"fixed\";\n+    let theme = \"selected\";\n+    // 한글 테마 👩‍💻\n+    render(theme);\n }\nadd: tests/settings.rs\n@@ -0,0 +1,4 @@\n+#[test]\n+fn selected_theme_is_retained() {\n+    assert_eq!(theme(), \"selected\");\n+}\n".into()),
        "diff" => (ActivityKind::AgentMessage, "```diff\n--- a/settings.rs\n+++ b/settings.rs\n@@ -1,2 +1,2 @@\n-let theme = \"fixed\";\n+let theme = \"selected\";\n render(theme);\n```".into()),
        "links" => (ActivityKind::AgentMessage, "## Links\n\nBare URL: (https://example.com/plain?a=1&amp;b=2).\n\n[Codex source](https://github.com/openai/codex) · [pi source](https://github.com/earendil-works/pi)\n\n[**한글** and `inline code`](https://example.com/docs) keep their destination when wrapped.\n\n| Source | Link |\n| --- | --- |\n| Rust | [Documentation](https://doc.rust-lang.org/book/) |\n\n```text\n[Literal code](https://example.com/not-linked)\n```\n\n[Local file stays text](file:///tmp/example.rs)\n\nSupported terminals can open web links. This fixture does not open a browser or fetch a page.".to_owned()),
        "markdown" => (ActivityKind::AgentMessage, "## Clearer output\n\n**Readable prose**, `inline code`, and lists.\n\n- Preserve your layout\n- Check 한글 and wrapping\n\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```\n\n> Ready to continue.".into()),
        "long" => (ActivityKind::AgentMessage, (1..=40).map(|n| format!("Line {n}: 한글과 English, wide characters and scrolling.  \n")).collect()),
        _ => (ActivityKind::AgentMessage, format!("받은 입력: {input}\n\n저는 화면 검증용 테스트 에이전트입니다. 실제 모델 호출 없이 응답을 스트리밍합니다.\n\n계속 입력해 대화를 이어가세요. tools · long-tools · error · long · markdown · tables · diff · changes · plan · usage · mcp · approval · interview 를 보내면 해당 UI를 확인할 수 있습니다. Esc로 중단할 수 있습니다.")),
    }
}

pub(in crate::runner) fn media_response(input: &str) -> Option<String> {
    match input {
        "footnotes" => Some("## Notes and sources\n\nOne claim[^source] and a second reference[^source].\n\n[^source]: Read [documentation](https://example.com/docs) and preserve `literal code`.\n\n    - Nested **detail** stays with this note.\n\nA Korean note[^출처].\n\n[^출처]: 한글 설명도 좁은 화면에서 줄바꿈합니다.\n\nUnresolved reference stays literal: [^missing].\n\n```text\n[^source]: this code stays literal\n```".into()),
        "syntax" => Some("## Code, with context\n\n```rust\n// Preserve indentation and comments\nfn greet(name: &str) -> String {\n    format!(\"Hello, {name}!\")\n}\n```\n\n```python\n# A small transformation\ndef totals(values):\n    return [value * 2 for value in values]\n```\n\n```json\n{\n  \"theme\": \"slate\",\n  \"enabled\": true,\n  \"retries\": 3\n}\n```".into()),
        "chart-series" => Some("## Shared-scale comparison\n\n```linechart\nheight: 6\nBaseline: 12 18 15 24 21\nCurrent: 8 12 22 18 30\nTarget: 20 20 20 20 20\n```\n\nNumbered points match the legend. Crosses mark plotted overlaps; mixed cells are muted.".into()),
        "chart-heights" => Some("## Compact trend\n\n```linechart\nheight: 3\n12 8 16 10 24\n```\n\n## Detailed relationship\n\n```scatterchart\nheight: 10\n0,0 1,5 10,10\n```".into()),
        "scatter" => Some("## Input size and latency\n\nX: kilobytes · Y: milliseconds\n\n```scatterchart\n4,12 8,18 16,17 32,26 64,31\n```\n\n## Fixed input size\n\n```scatterchart\n5,10 5,20 5,30\n```".into()),
        "histogram" => Some("## Request latency distribution\n\nMilliseconds · interval counts\n\n```histogram\nbins: 4\n12 14 15 15 18 20 21 22 25 27 29 32\n```\n\n## Constant values\n\n```histogram\n5 5 5\n```".into()),
        "charts" => Some("## Rendering checks\n\nMeasured examples · milliseconds\n\n```chart\nMarkdown: 18\nTables: 12\nCode: 24\nImages: 31\n```\n\n## Latency over time\n\n```linechart\n31 27 29 22 18 21 16 12\n```\n\n## Concurrent tasks\n\n```stepchart\n1 1 4 4 2 2 0\n```\n\n## Change from baseline\n\n```chart\nBefore: -12\nSame: 0\nAfter: 8\n```\n\n## Scientific notation\n\n```linechart\n0 1e308 -1e308\n```\n\n## Very small values\n\n```sparkline\n0 5e-324\n```\n\n## Numeric notation\n\n```chart\nTiny: 5e-324\nDecimal: 1.2300\nSigned: +2.00\nZero: -0.0\n```".into()),
        "images" => Some(sample_image()),
        "message-image" => MessageContent {
            block: json!({"type":"image","mimeType":"image/png","data":sample_image_data(),"_meta":{"source":"offline answer block"}}),
        }.to_snapshot(),
        "image-orientation" => Some(sample_oriented_image()),
        "media-errors" => Some("## Useful fallbacks\n\n![Incomplete image](data:image/png;base64,broken)\n\n![External reference](https://example.com/diagram.png)\n\n```chart\nReading: pending\n```\n\n```unknown-language\nOriginal source stays readable.\n```".into()),
        "showcase" => Some(["syntax", "charts", "images", "media-errors"].into_iter().filter_map(media_response).collect::<Vec<_>>().join("\n\n")),
        _ => input.strip_prefix("image ").map(local_image),
    }
}

fn sample_batch_read() -> String {
    let result = json!({"results":[
        {"path":"src/main.rs","status":"ok","start":2,"end":4,"total":9,"next_offset":5,"content":"fn main() {\n    println!(\"batch preview\");\n}\n"},
        {"path":"missing.txt","status":"error","error":"unavailable"},
        {"path":"empty.txt","status":"ok","start":0,"end":0,"total":0,"content":""}
    ]}).to_string();
    ToolOutput {
        tool:"read_files".to_owned(), server:Some("preview".to_owned()),
        arguments:Some(json!({"files":[{"path":"src/main.rs"},{"path":"missing.txt"},{"path":"empty.txt"}]})),
        result:Some(json!({"content":[{"type":"text","text":result}],"note":"Offline fixture: no files were read."})),
        content_items:None, error:None,
        plain_text:format!("preview.read_files\n{result}\nOffline fixture: no files were read."),
    }.to_snapshot().expect("bounded batch read fixture")
}

fn sample_tool_image() -> String {
    let data = sample_image_data();
    ToolOutput {
        tool: "capture".to_owned(), server: Some("preview".to_owned()),
        arguments: Some("offline fixture".into()),
        result: Some(format!(r#"{{"content":[{{"type":"text","text":"Preview only: no external tool was called."}},{{"type":"image","mimeType":"image/png","data":"{data}"}}]}}"#).parse().expect("fixture result JSON")),
        content_items: None, error: None,
        plain_text: "preview.capture\nOffline PNG image · 128 × 64".to_owned(),
    }.to_snapshot().expect("bounded tool output fixture")
}

fn sample_image() -> String {
    format!(
        "## Image in the conversation\n\n![Mountain study](data:image/png;base64,{})\n\nThe image scrolls with the answer. Try resizing the pane.\n\nUse `image /absolute/path.png` here to inspect your own local PNG or JPEG.",
        sample_image_data()
    )
}

fn sample_oriented_image() -> String {
    let upright = RgbImage::from_fn(64, 128, |x, y| {
        if (i64::from(x) - 46).pow(2) + (i64::from(y) - 22).pow(2) < 90 {
            Rgb([244, 204, 119])
        } else if y > 75 {
            Rgb([38, 103, 102])
        } else {
            Rgb([62, 86, 140])
        }
    });
    let mut stored = Cursor::new(Vec::new());
    rotate270(&upright)
        .write_to(&mut stored, ImageFormat::Jpeg)
        .expect("small JPEG fixture");
    let stored = stored.into_inner();
    // Little-endian TIFF Orientation=6: restore the stored landscape to upright portrait.
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut jpeg = stored[..2].to_vec();
    jpeg.extend_from_slice(&[0xff, 0xe1]);
    jpeg.extend_from_slice(&u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
    jpeg.extend_from_slice(exif);
    jpeg.extend_from_slice(&stored[2..]);
    format!(
        "## JPEG orientation\n\n![Upright portrait](data:image/jpeg;base64,{})\n\nEXIF rotates the stored 128 x 64 JPEG to 64 x 128. The sun belongs at the top right, ground at the bottom.",
        STANDARD.encode(jpeg)
    )
}

fn sample_image_data() -> String {
    // A deterministic landscape fixture is encoded as a real PNG, then follows
    // exactly the same decoder and cell renderer as an attached image.
    let image = RgbImage::from_fn(128, 64, |x, y| {
        let sky = Rgb([35 + (y * 2) as u8, 66 + y as u8, 102 + y as u8]);
        if (i64::from(x) - 95).pow(2) + (i64::from(y) - 17).pow(2) < 70 {
            Rgb([244, 204, 119])
        } else if y > (38.0 + (f64::from(x) / 12.0).sin() * 8.0) as u32 {
            Rgb([38, 103, 102])
        } else if y > (29.0 + (f64::from(x) / 17.0).sin() * 9.0) as u32 {
            Rgb([62, 86, 110])
        } else {
            sky
        }
    });
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, ImageFormat::Png)
        .expect("small in-memory PNG fixture");
    STANDARD.encode(bytes.into_inner())
}

fn local_image(path: &str) -> String {
    let path = path.trim();
    let load = || -> io::Result<Vec<u8>> {
        let metadata = fs::metadata(path)?;
        if !metadata.is_file() || metadata.len() > 1_048_576 {
            return Err(io::Error::other(
                "choose a regular PNG/JPEG file no larger than 1 MiB",
            ));
        }
        let mut bytes = Vec::new();
        File::open(path)?.take(1_048_577).read_to_end(&mut bytes)?;
        if bytes.len() > 1_048_576 {
            return Err(io::Error::other("image exceeds 1 MiB"));
        }
        Ok(bytes)
    };
    match load() {
        Ok(bytes) => format!(
            "![Local image](data:image/png;base64,{})",
            STANDARD.encode(bytes)
        ),
        Err(error) => format!(
            "Image preview unavailable.\n\n{error}\n\nTry `image /absolute/path.png` with a readable PNG or JPEG."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 인터뷰는 첫 답변 뒤 새 요청을 발행하고 두 번째 답변 뒤에만 Turn을 끝낸다.
    #[test]
    fn interview_preserves_both_answers_and_correlations() {
        let mut agent = TestAgent::new();
        agent
            .dispatch(AgentAction::submit("interview").unwrap())
            .unwrap();
        let Some(PreviewRequest::Interview { request: first, .. }) = agent.request else {
            panic!("first question missing")
        };
        agent
            .dispatch(AgentAction::RespondToUserInput {
                request: first,
                input: "reading".into(),
            })
            .unwrap();
        let Some(PreviewRequest::Interview {
            request: second, ..
        }) = agent.request
        else {
            panic!("second question missing")
        };
        assert_ne!(first, second);
        agent
            .dispatch(AgentAction::RespondToUserInput {
                request: first,
                input: "stale".into(),
            })
            .unwrap();
        assert!(
            matches!(agent.request, Some(PreviewRequest::Interview { request, .. }) if request == second)
        );
        agent
            .dispatch(AgentAction::RespondToUserInput {
                request: second,
                input: "compact".into(),
            })
            .unwrap();
        assert!(agent.request.is_none());
        assert!(agent.active.is_none());
        assert!(agent.ready.iter().any(|event| matches!(event,
            AgentPoll::Record(TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text), ..
            })) if text.contains("Priority: reading") && text.contains("Density: compact") && !text.contains("stale"))));
    }

    // 첫 질문 취소와 부분 답변 뒤 취소 모두 실제 기록 수만 한 번 표시한다.
    #[test]
    fn interrupted_interview_reports_recorded_and_unanswered_questions_once() {
        for recorded in [false, true] {
            let mut agent = TestAgent::new();
            agent
                .dispatch(AgentAction::submit("interview").unwrap())
                .unwrap();
            if recorded {
                let Some(PreviewRequest::Interview { request, .. }) = agent.request else {
                    panic!("question missing")
                };
                agent
                    .dispatch(AgentAction::RespondToUserInput {
                        request,
                        input: "1".into(),
                    })
                    .unwrap();
            }
            agent.dispatch(AgentAction::Interrupt).unwrap();
            let summaries = agent
                .ready
                .iter()
                .filter_map(|event| {
                    if let AgentPoll::Record(TranscriptRecord::EventCommitted(
                        AgentEvent::ActivityUpdated {
                            update: ActivityUpdate::TextSnapshot(text),
                            ..
                        },
                    )) = event
                    {
                        ActivityNotice::from_snapshot(text)
                            .filter(|notice| notice.title == "Interview incomplete")
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(summaries.len(), 1);
            assert_eq!(summaries[0].level, NoticeLevel::Warning);
            assert!(summaries[0].message.contains(if recorded {
                "1/2 answers recorded · 1 unanswered"
            } else {
                "0/2 answers recorded · 2 unanswered"
            }));
            assert!(
                summaries[0]
                    .message
                    .contains("2. Unanswered: How much detail should stay visible?")
            );
            assert!(summaries[0].message.contains("Offline preview"));
            assert!(agent.request.is_none());
            assert!(agent.active.is_none());
            let before = agent.ready.len();
            agent.dispatch(AgentAction::Interrupt).unwrap();
            assert_eq!(agent.ready.len(), before);
        }
    }

    // idle preview는 polling timer를 요구하지 않는다. 합성 출력이 있을 때만 깨우고,
    // 중단 후 종료 observation을 소비하면 다시 무기한 입력 대기로 돌아간다.
    #[test]
    fn preview_deadlines_exist_only_while_events_are_pending() {
        let mut agent = TestAgent::new();
        assert_eq!(agent.next_deadline(), None);
        agent
            .dispatch(AgentAction::submit("long").unwrap())
            .unwrap();
        assert!(agent.next_deadline().is_some());
        while !agent.ready.is_empty() {
            agent.poll().unwrap();
        }
        agent.next = Instant::now() + Duration::from_secs(1);
        assert_eq!(agent.next_deadline(), Some(agent.next));
        agent.dispatch(AgentAction::Interrupt).unwrap();
        while !agent.ready.is_empty() {
            agent.poll().unwrap();
        }
        assert_eq!(agent.next_deadline(), None);
    }
    // 세션 경고 예시는 입력만 수락하고 가짜 journal record·Turn·stream을 생성하지 않는다.
    #[test]
    fn session_notice_preview_has_no_turn_or_journal_record() {
        for (input, title) in [
            ("warning", "Codex configuration warning"),
            ("deprecation", "Codex deprecation notice"),
            ("approval-warning", "Codex approval warning"),
        ] {
            let mut agent = TestAgent::new();
            agent.dispatch(AgentAction::submit(input).unwrap()).unwrap();
            assert!(matches!(agent.poll().unwrap(), AgentPoll::Submission(_)));
            let AgentPoll::Notice(notice) = agent.poll().unwrap() else {
                panic!("missing session notice");
            };
            assert_eq!(notice.title, title);
            assert!(notice.message.starts_with("Preview only:"));
            assert_eq!(agent.poll().unwrap(), AgentPoll::Pending);
            assert_eq!(agent.turns, 0);
            assert!(agent.stream.is_empty());
            assert!(agent.active.is_none());
        }
    }

    // 입력을 수락하고 실제 문자열을 응답에 반영하며, 중단 후 새 대화를 받을 수 있어야 한다.
    #[test]
    fn accepts_interrupts_and_accepts_again() {
        let mut agent = TestAgent::new();
        agent
            .dispatch(AgentAction::submit("hello").unwrap())
            .unwrap();
        assert!(matches!(
            agent.poll().unwrap(),
            AgentPoll::Submission(SubmissionOutcome::Accepted { .. })
        ));
        assert!(
            agent
                .stream
                .iter()
                .any(|event| matches!(event, AgentEvent::ActivityUpdated {
            update: ActivityUpdate::TextDelta(text), .. } if text.contains("hello")))
        );
        agent.dispatch(AgentAction::Interrupt).unwrap();
        assert!(agent.stream.is_empty());
        assert!(agent.active.is_none());
        assert_eq!(
            agent
                .dispatch(AgentAction::submit("again").unwrap())
                .unwrap(),
            DispatchOutcome::Queued
        );
    }
    // 작업 중 입력은 조용히 유실시키지 않고 거절하여 frontend의 draft 보존 경로로 돌린다.
    #[test]
    fn busy_input_is_rejected() {
        let mut agent = TestAgent::new();
        agent
            .dispatch(AgentAction::submit("long").unwrap())
            .unwrap();
        assert!(matches!(
            agent
                .dispatch(AgentAction::submit("keep draft").unwrap())
                .unwrap(),
            DispatchOutcome::Rejected { .. }
        ));
    }

    // 모의 실패는 연결을 닫지 않고 Turn을 끝내 다음 입력으로 복구할 수 있어야 한다.
    #[test]
    fn failed_turn_finishes_and_allows_recovery() {
        let mut agent = TestAgent::new();
        agent
            .dispatch(AgentAction::submit("error").unwrap())
            .unwrap();
        let mut failed = false;
        for _ in 0..200 {
            agent.next = Instant::now();
            if matches!(
                agent.poll().unwrap(),
                AgentPoll::Record(TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                    outcome: TurnOutcome::Failed(_),
                    ..
                }))
            ) {
                failed = true;
                break;
            }
        }
        assert!(failed);
        assert!(agent.active.is_none());
        assert_eq!(
            agent
                .dispatch(AgentAction::submit("recover").unwrap())
                .unwrap(),
            DispatchOutcome::Queued
        );
    }
    // 구조화 이미지 예시는 불완전 JSON delta 대신 한 개의 완전한 snapshot으로 전달한다.
    #[test]
    fn tool_image_preview_emits_one_structured_snapshot() {
        let mut agent = TestAgent::new();
        agent
            .dispatch(AgentAction::submit("mcp-image").unwrap())
            .unwrap();
        assert!(agent.stream.is_empty());
        let updates = agent
            .ready
            .iter()
            .filter_map(|event| match event {
                AgentPoll::Record(TranscriptRecord::EventCommitted(
                    AgentEvent::ActivityUpdated { update, .. },
                )) => Some(update),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(updates.len(), 1);
        let ActivityUpdate::TextSnapshot(text) = updates[0] else {
            panic!("complete snapshot required")
        };
        let output = ToolOutput::from_snapshot(text).unwrap();
        assert_eq!(output.tool, "capture");
        assert!(
            output
                .content_blocks()
                .any(|block| block.get("type").is_some_and(|kind| kind == "image"))
        );
    }
    // 상태 예시는 전체 snapshot을 갱신·삭제하되 Turn이나 대화 기록을 만들지 않는다.
    #[test]
    fn host_status_preview_stays_outside_conversation_events() {
        let mut agent = TestAgent::new();
        for (command, expected) in [
            (
                "status",
                "Demo branch: chat-ui · Demo checks: 18 passed · Demo worker: idle",
            ),
            (
                "status-update",
                "Demo checks: 20 passed · Demo worker: ready",
            ),
            ("status-clear", ""),
        ] {
            agent
                .dispatch(AgentAction::submit(command).unwrap())
                .unwrap();
            assert!(matches!(
                agent.poll().unwrap(),
                AgentPoll::Submission(SubmissionOutcome::Accepted { .. })
            ));
            let AgentPoll::StatusLine(status) = agent.poll().unwrap() else {
                panic!("status snapshot missing")
            };
            assert_eq!(status.as_str(), expected);
            assert_eq!(agent.poll().unwrap(), AgentPoll::Pending);
            assert!(agent.active.is_none());
            assert_eq!(agent.turns, 0);
        }
    }
    // 승인은 응답 전 대기하며 다른 요청의 결정은 무시하고 구조화된 거절도 정확한 결과 제목을
    // 표시한다.
    #[test]
    fn approval_preview_waits_for_exact_request_and_labels_offered_decline() {
        for command in ["approval", "approval-scopes", "approval-diff"] {
            for declined in [false, true] {
                let mut agent = TestAgent::new();
                agent
                    .dispatch(AgentAction::submit(command).unwrap())
                    .unwrap();
                let Some(PreviewRequest::Approval(request, kind)) = agent.request else {
                    panic!("approval request missing")
                };
                let decision = match kind.profile() {
                    Some(profile) => ApprovalDecision::Offered(if declined {
                        profile.decline_choice.unwrap()
                    } else {
                        1
                    }),
                    None => {
                        if declined {
                            ApprovalDecision::Declined
                        } else {
                            ApprovalDecision::Approved
                        }
                    },
                };
                while !agent.ready.is_empty() {
                    agent.poll().unwrap();
                }
                for _ in 0..3 {
                    assert_eq!(agent.poll().unwrap(), AgentPoll::Pending);
                }
                assert!(agent.stream.is_empty());
                let stale = ActivityRequestRef::new(
                    request.activity(),
                    RequestId::new(NonZeroU64::new(999).unwrap()),
                );
                agent
                    .dispatch(AgentAction::RespondToApproval {
                        request: stale,
                        decision,
                    })
                    .unwrap();
                assert!(
                    matches!(agent.request, Some(PreviewRequest::Approval(current, _)) if current == request)
                );
                assert_eq!(agent.poll().unwrap(), AgentPoll::Pending);
                agent
                    .dispatch(AgentAction::RespondToApproval { request, decision })
                    .unwrap();
                assert!(agent.request.is_none());
                let snapshots: Vec<_> = agent
                    .ready
                    .iter()
                    .filter_map(|event| match event {
                        AgentPoll::Record(TranscriptRecord::EventCommitted(
                            AgentEvent::ActivityUpdated {
                                update: ActivityUpdate::TextSnapshot(text),
                                ..
                            },
                        )) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    snapshots
                        .iter()
                        .filter(|text| text.starts_with("Decision:"))
                        .count(),
                    1
                );
                assert_eq!(
                    snapshots.iter().any(|text| text.starts_with("## Declined")),
                    declined
                );
                assert_eq!(
                    snapshots
                        .iter()
                        .any(|text| text.starts_with("## Approval recorded")),
                    !declined
                );
                let count = agent.ready.len();
                agent
                    .dispatch(AgentAction::RespondToApproval { request, decision })
                    .unwrap();
                assert_eq!(agent.ready.len(), count);
            }
        }
    }
    // 세션 문서 예시는 사용자 제출 수락과 문서만 내보내고 Turn·저널 record를 만들지 않는다.
    #[test]
    fn session_document_preview_emits_only_host_document() {
        let mut agent = TestAgent::new();
        agent
            .dispatch(AgentAction::submit("session-document").unwrap())
            .unwrap();
        assert!(matches!(
            agent.poll().unwrap(),
            AgentPoll::Submission(SubmissionOutcome::Accepted { .. })
        ));
        let AgentPoll::Document(document) = agent.poll().unwrap() else {
            panic!("host document missing")
        };
        let profile = ActivityDocument::from_snapshot(document.snapshot()).unwrap();
        assert_eq!(profile.title, "Workspace guide");
        assert!(profile.markdown.contains("| Action | Shortcut |"));
        assert_eq!(agent.poll().unwrap(), AgentPoll::Pending);
        assert_eq!(agent.turns, 0);
        assert!(agent.active.is_none());
    }
}
