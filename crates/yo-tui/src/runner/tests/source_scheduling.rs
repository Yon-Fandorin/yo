use std::{
    convert::Infallible,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

use crossterm::event::Event;

use crate::{
    input::event::InputEvent,
    runner::{
        AgentAction, AgentConnection, AgentPoll, DispatchOutcome, PendingDispatch,
        SkillReferenceConnection, SkillReferencePoll, TerminationEvent, TerminationSource,
        WorkspaceReferenceConnection, WorkspaceReferencePoll,
        source_schedule::{OrdinarySource, SourceSchedule},
        state::{StateEffect, TuiState},
        unix::{
            LoopError, OrdinaryObservation, OrdinaryPoll, apply_agent_poll, apply_skill_poll,
            apply_workspace_poll, poll_ordinary,
        },
    },
    terminal::backend::unix::{EventSource, UnixEventReader},
};

struct ReadyEvents {
    observations: Arc<AtomicUsize>,
}

impl EventSource for ReadyEvents {
    type Error = io::Error;

    fn poll_event(&mut self, _context: &mut Context<'_>) -> Poll<Result<Event, Self::Error>> {
        self.observations.fetch_add(1, Ordering::Relaxed);
        Poll::Ready(Ok(Event::Resize(80, 24)))
    }
}

struct PendingEvents {
    polls: Arc<AtomicUsize>,
}

impl PendingEvents {
    fn uncounted() -> Self {
        Self {
            polls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl EventSource for PendingEvents {
    type Error = io::Error;

    fn poll_event(&mut self, _context: &mut Context<'_>) -> Poll<Result<Event, Self::Error>> {
        self.polls.fetch_add(1, Ordering::Relaxed);
        Poll::Pending
    }
}

struct FailingEvents;

impl EventSource for FailingEvents {
    type Error = &'static str;

    fn poll_event(&mut self, _context: &mut Context<'_>) -> Poll<Result<Event, Self::Error>> {
        Poll::Ready(Err("terminal disconnected"))
    }
}

struct PendingAgent {
    readiness_polls: Arc<AtomicUsize>,
}

impl PendingAgent {
    fn uncounted() -> Self {
        Self {
            readiness_polls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl AgentConnection for PendingAgent {
    type Error = Infallible;

    fn dispatch(&mut self, _action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn retry(&mut self, _pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        panic!("a pending source must not be consumed")
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        self.readiness_polls.fetch_add(1, Ordering::Relaxed);
        Poll::Pending
    }
}

struct ReadyAgent {
    observations: Arc<AtomicUsize>,
}

impl AgentConnection for ReadyAgent {
    type Error = Infallible;

    fn dispatch(&mut self, _action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn retry(&mut self, _pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        self.observations.fetch_add(1, Ordering::Relaxed);
        Ok(AgentPoll::Closed)
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }
}

struct ReadyReferences {
    observations: Arc<AtomicUsize>,
}

impl WorkspaceReferenceConnection for ReadyReferences {
    fn search(&mut self, _request: yo_core::WorkspaceReferenceSearchRequest) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> Result<WorkspaceReferencePoll, String> {
        self.observations.fetch_add(1, Ordering::Relaxed);
        Err("workspace disconnected".to_owned())
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }
}

impl SkillReferenceConnection for ReadyReferences {
    fn search(&mut self, _request: yo_core::SkillReferenceSearchRequest) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> Result<SkillReferencePoll, String> {
        self.observations.fetch_add(1, Ordering::Relaxed);
        Err("skill disconnected".to_owned())
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }
}

struct PendingReferences {
    readiness_polls: Arc<AtomicUsize>,
}

impl WorkspaceReferenceConnection for PendingReferences {
    fn search(&mut self, _request: yo_core::WorkspaceReferenceSearchRequest) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> Result<WorkspaceReferencePoll, String> {
        panic!("a pending workspace source must not be consumed")
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        self.readiness_polls.fetch_add(1, Ordering::Relaxed);
        Poll::Pending
    }
}

impl SkillReferenceConnection for PendingReferences {
    fn search(&mut self, _request: yo_core::SkillReferenceSearchRequest) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> Result<SkillReferencePoll, String> {
        panic!("a pending skill source must not be consumed")
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        self.readiness_polls.fetch_add(1, Ordering::Relaxed);
        Poll::Pending
    }
}

struct NeverTerminate;

impl TerminationSource for NeverTerminate {
    fn poll_termination(&mut self, _context: &mut Context<'_>) -> Poll<TerminationEvent> {
        Poll::Pending
    }
}

struct AlreadyTerminated;

impl TerminationSource for AlreadyTerminated {
    fn poll_termination(&mut self, _context: &mut Context<'_>) -> Poll<TerminationEvent> {
        Poll::Ready(TerminationEvent::Requested)
    }
}

struct CountingPendingTermination {
    polls: Arc<AtomicUsize>,
}

impl TerminationSource for CountingPendingTermination {
    fn poll_termination(&mut self, _context: &mut Context<'_>) -> Poll<TerminationEvent> {
        self.polls.fetch_add(1, Ordering::Relaxed);
        Poll::Pending
    }
}

struct TerminateAfterObservation {
    observations: Arc<AtomicUsize>,
}

impl TerminationSource for TerminateAfterObservation {
    fn poll_termination(&mut self, _context: &mut Context<'_>) -> Poll<TerminationEvent> {
        if self.observations.load(Ordering::Relaxed) == 0 {
            Poll::Pending
        } else {
            Poll::Ready(TerminationEvent::Requested)
        }
    }
}

#[derive(Default)]
struct RearmState {
    readiness_polls: AtomicUsize,
    registered: Mutex<Option<Waker>>,
}

impl RearmState {
    fn poll_ready(&self, context: &mut Context<'_>) -> Poll<()> {
        if self.readiness_polls.fetch_add(1, Ordering::Relaxed) == 0 {
            Poll::Ready(())
        } else {
            *self.registered.lock().unwrap() = Some(context.waker().clone());
            Poll::Pending
        }
    }

    fn wake_registered(&self) {
        self.registered.lock().unwrap().take().unwrap().wake();
    }
}

struct RearmAgent {
    state: Arc<RearmState>,
}

impl AgentConnection for RearmAgent {
    type Error = Infallible;

    fn dispatch(&mut self, _action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn retry(&mut self, _pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(AgentPoll::Pending)
    }

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        self.state.poll_ready(context)
    }
}

struct RearmReferences {
    state: Arc<RearmState>,
}

impl WorkspaceReferenceConnection for RearmReferences {
    fn search(&mut self, _request: yo_core::WorkspaceReferenceSearchRequest) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> Result<WorkspaceReferencePoll, String> {
        Ok(WorkspaceReferencePoll::Pending)
    }

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        self.state.poll_ready(context)
    }
}

impl SkillReferenceConnection for RearmReferences {
    fn search(&mut self, _request: yo_core::SkillReferenceSearchRequest) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> Result<SkillReferencePoll, String> {
        Ok(SkillReferencePoll::Pending)
    }

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        self.state.poll_ready(context)
    }
}

struct CountingWake(AtomicUsize);

impl Wake for CountingWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

struct AlwaysReadyPendingAgent;

impl AgentConnection for AlwaysReadyPendingAgent {
    type Error = Infallible;

    fn dispatch(&mut self, _action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn retry(&mut self, _pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(AgentPoll::Pending)
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }
}

// 이미 게시된 종료 요청은 source cursor보다 먼저 선택되어 ordinary source를 한 번도
// poll하지 않는다.
#[test]
fn published_termination_precedes_source_selection() {
    let terminal_polls = Arc::new(AtomicUsize::new(0));
    let agent_polls = Arc::new(AtomicUsize::new(0));
    let mut events = UnixEventReader::new(
        PendingEvents {
            polls: Arc::clone(&terminal_polls),
        },
        AlreadyTerminated,
    );
    let mut agent = PendingAgent {
        readiness_polls: Arc::clone(&agent_polls),
    };
    let mut workspace = None;
    let mut skill = None;
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert!(matches!(
        poll_ordinary(
            &mut events,
            &mut agent,
            &mut workspace,
            &mut skill,
            &SourceSchedule::default(),
            &mut context,
        )
        .unwrap(),
        OrdinaryPoll::Termination
    ));
    assert_eq!(terminal_polls.load(Ordering::Relaxed), 0);
    assert_eq!(agent_polls.load(Ordering::Relaxed), 0);
}

// 네 ordinary source가 동시에 계속 ready여도 cursor는 observation 하나마다 successor로
// 이동해 두 순회 동안 정확히 같은 대칭 순서와 source별 payload를 보존한다.
#[test]
fn continuously_ready_sources_are_selected_one_at_a_time_in_cyclic_order() {
    let observations = Arc::new(AtomicUsize::new(0));
    let mut events = UnixEventReader::new(
        ReadyEvents {
            observations: Arc::clone(&observations),
        },
        NeverTerminate,
    );
    let mut agent = ReadyAgent {
        observations: Arc::clone(&observations),
    };
    let mut workspace: Option<Box<dyn WorkspaceReferenceConnection>> =
        Some(Box::new(ReadyReferences {
            observations: Arc::clone(&observations),
        }));
    let mut skill: Option<Box<dyn SkillReferenceConnection>> = Some(Box::new(ReadyReferences {
        observations: Arc::clone(&observations),
    }));
    let mut schedule = SourceSchedule::default();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut selected = Vec::new();

    for _ in 0..8 {
        let OrdinaryPoll::Ready {
            source,
            observation,
        } = poll_ordinary(
            &mut events,
            &mut agent,
            &mut workspace,
            &mut skill,
            &schedule,
            &mut context,
        )
        .unwrap()
        else {
            panic!("one continuously-ready source must be selected");
        };
        match (source, observation) {
            (OrdinarySource::Terminal, OrdinaryObservation::Input(_))
            | (OrdinarySource::Agent, OrdinaryObservation::Agent(_)) => {},
            (OrdinarySource::Workspace, OrdinaryObservation::Workspace(Err(error))) => {
                assert_eq!(error, "workspace disconnected");
            },
            (OrdinarySource::Skill, OrdinaryObservation::Skill(Err(error))) => {
                assert_eq!(error, "skill disconnected");
            },
            _ => panic!("selected source must retain its matching observation"),
        }
        selected.push(source);
        schedule.handled(source);
    }

    assert_eq!(
        selected,
        [
            OrdinarySource::Terminal,
            OrdinarySource::Agent,
            OrdinarySource::Workspace,
            OrdinarySource::Skill,
            OrdinarySource::Terminal,
            OrdinarySource::Agent,
            OrdinarySource::Workspace,
            OrdinarySource::Skill,
        ]
    );
    assert_eq!(observations.load(Ordering::Relaxed), 8);
}

// 어떤 source도 ready가 아니면 wait 전에 termination과 네 live ordinary source를 모두
// Context로 poll해 각 producer가 owner-thread waker를 등록할 기회를 얻는다.
#[test]
fn pending_selection_registers_every_live_source_before_waiting() {
    let terminal_polls = Arc::new(AtomicUsize::new(0));
    let agent_polls = Arc::new(AtomicUsize::new(0));
    let workspace_polls = Arc::new(AtomicUsize::new(0));
    let skill_polls = Arc::new(AtomicUsize::new(0));
    let termination_polls = Arc::new(AtomicUsize::new(0));
    let mut events = UnixEventReader::new(
        PendingEvents {
            polls: Arc::clone(&terminal_polls),
        },
        CountingPendingTermination {
            polls: Arc::clone(&termination_polls),
        },
    );
    let mut agent = PendingAgent {
        readiness_polls: Arc::clone(&agent_polls),
    };
    let mut workspace: Option<Box<dyn WorkspaceReferenceConnection>> =
        Some(Box::new(PendingReferences {
            readiness_polls: Arc::clone(&workspace_polls),
        }));
    let mut skill: Option<Box<dyn SkillReferenceConnection>> = Some(Box::new(PendingReferences {
        readiness_polls: Arc::clone(&skill_polls),
    }));
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert!(matches!(
        poll_ordinary(
            &mut events,
            &mut agent,
            &mut workspace,
            &mut skill,
            &SourceSchedule::default(),
            &mut context,
        )
        .unwrap(),
        OrdinaryPoll::Pending
    ));
    assert_eq!(terminal_polls.load(Ordering::Relaxed), 1);
    assert_eq!(agent_polls.load(Ordering::Relaxed), 1);
    assert_eq!(workspace_polls.load(Ordering::Relaxed), 1);
    assert_eq!(skill_polls.load(Ordering::Relaxed), 1);
    assert_eq!(termination_polls.load(Ordering::Relaxed), 5);
}

// terminal·agent·workspace·skill 각각의 poll이 observation을 만든 바로 뒤 종료가
// 게시되면 cursor result를 반환하지 않고 termination이 동일하게 우선한다.
#[test]
fn termination_after_each_ordinary_poll_discards_that_observation() {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    let terminal_observations = Arc::new(AtomicUsize::new(0));
    let mut terminal_events = UnixEventReader::new(
        ReadyEvents {
            observations: Arc::clone(&terminal_observations),
        },
        TerminateAfterObservation {
            observations: Arc::clone(&terminal_observations),
        },
    );
    let mut pending_agent = PendingAgent::uncounted();
    let mut no_workspace = None;
    let mut no_skill = None;
    assert!(matches!(
        poll_ordinary(
            &mut terminal_events,
            &mut pending_agent,
            &mut no_workspace,
            &mut no_skill,
            &SourceSchedule::default(),
            &mut context,
        )
        .unwrap(),
        OrdinaryPoll::Termination
    ));

    let agent_observations = Arc::new(AtomicUsize::new(0));
    let mut agent_events = UnixEventReader::new(
        PendingEvents::uncounted(),
        TerminateAfterObservation {
            observations: Arc::clone(&agent_observations),
        },
    );
    let mut ready_agent = ReadyAgent {
        observations: Arc::clone(&agent_observations),
    };
    let mut agent_schedule = SourceSchedule::default();
    agent_schedule.handled(OrdinarySource::Terminal);
    assert!(matches!(
        poll_ordinary(
            &mut agent_events,
            &mut ready_agent,
            &mut no_workspace,
            &mut no_skill,
            &agent_schedule,
            &mut context,
        )
        .unwrap(),
        OrdinaryPoll::Termination
    ));

    let workspace_observations = Arc::new(AtomicUsize::new(0));
    let mut workspace_events = UnixEventReader::new(
        PendingEvents::uncounted(),
        TerminateAfterObservation {
            observations: Arc::clone(&workspace_observations),
        },
    );
    let mut workspace_agent = PendingAgent::uncounted();
    let mut workspace: Option<Box<dyn WorkspaceReferenceConnection>> =
        Some(Box::new(ReadyReferences {
            observations: Arc::clone(&workspace_observations),
        }));
    let mut workspace_schedule = SourceSchedule::default();
    workspace_schedule.handled(OrdinarySource::Agent);
    assert!(matches!(
        poll_ordinary(
            &mut workspace_events,
            &mut workspace_agent,
            &mut workspace,
            &mut no_skill,
            &workspace_schedule,
            &mut context,
        )
        .unwrap(),
        OrdinaryPoll::Termination
    ));

    let skill_observations = Arc::new(AtomicUsize::new(0));
    let mut skill_events = UnixEventReader::new(
        PendingEvents::uncounted(),
        TerminateAfterObservation {
            observations: Arc::clone(&skill_observations),
        },
    );
    let mut skill_agent = PendingAgent::uncounted();
    let mut skill: Option<Box<dyn SkillReferenceConnection>> = Some(Box::new(ReadyReferences {
        observations: Arc::clone(&skill_observations),
    }));
    let mut skill_schedule = SourceSchedule::default();
    skill_schedule.handled(OrdinarySource::Workspace);
    assert!(matches!(
        poll_ordinary(
            &mut skill_events,
            &mut skill_agent,
            &mut no_workspace,
            &mut skill,
            &skill_schedule,
            &mut context,
        )
        .unwrap(),
        OrdinaryPoll::Termination
    ));
}

// agent·workspace·skill이 Ready 직후 semantic Pending을 반환하면 같은 Context로 readiness를
// 다시 등록하며, 이후 producer wake가 owner waker까지 실제로 전달된다.
#[test]
fn ready_then_pending_rearms_every_two_phase_source_before_waiting() {
    let agent_state = Arc::new(RearmState::default());
    let agent_wake = Arc::new(CountingWake(AtomicUsize::new(0)));
    let agent_waker = Waker::from(Arc::clone(&agent_wake));
    let mut agent_context = Context::from_waker(&agent_waker);
    let mut agent_events = UnixEventReader::new(PendingEvents::uncounted(), NeverTerminate);
    let mut agent = RearmAgent {
        state: Arc::clone(&agent_state),
    };
    let mut no_workspace = None;
    let mut no_skill = None;
    assert!(matches!(
        poll_ordinary(
            &mut agent_events,
            &mut agent,
            &mut no_workspace,
            &mut no_skill,
            &SourceSchedule::default(),
            &mut agent_context,
        )
        .unwrap(),
        OrdinaryPoll::Pending
    ));
    assert_eq!(agent_state.readiness_polls.load(Ordering::Relaxed), 2);
    agent_state.wake_registered();
    assert_eq!(agent_wake.0.load(Ordering::Relaxed), 1);

    let workspace_state = Arc::new(RearmState::default());
    let workspace_wake = Arc::new(CountingWake(AtomicUsize::new(0)));
    let workspace_waker = Waker::from(Arc::clone(&workspace_wake));
    let mut workspace_context = Context::from_waker(&workspace_waker);
    let mut workspace_events = UnixEventReader::new(PendingEvents::uncounted(), NeverTerminate);
    let mut workspace_agent = PendingAgent::uncounted();
    let mut workspace: Option<Box<dyn WorkspaceReferenceConnection>> =
        Some(Box::new(RearmReferences {
            state: Arc::clone(&workspace_state),
        }));
    let mut workspace_schedule = SourceSchedule::default();
    workspace_schedule.handled(OrdinarySource::Agent);
    assert!(matches!(
        poll_ordinary(
            &mut workspace_events,
            &mut workspace_agent,
            &mut workspace,
            &mut no_skill,
            &workspace_schedule,
            &mut workspace_context,
        )
        .unwrap(),
        OrdinaryPoll::Pending
    ));
    assert_eq!(workspace_state.readiness_polls.load(Ordering::Relaxed), 2);
    workspace_state.wake_registered();
    assert_eq!(workspace_wake.0.load(Ordering::Relaxed), 1);

    let skill_state = Arc::new(RearmState::default());
    let skill_wake = Arc::new(CountingWake(AtomicUsize::new(0)));
    let skill_waker = Waker::from(Arc::clone(&skill_wake));
    let mut skill_context = Context::from_waker(&skill_waker);
    let mut skill_events = UnixEventReader::new(PendingEvents::uncounted(), NeverTerminate);
    let mut skill_agent = PendingAgent::uncounted();
    let mut skill: Option<Box<dyn SkillReferenceConnection>> = Some(Box::new(RearmReferences {
        state: Arc::clone(&skill_state),
    }));
    let mut skill_schedule = SourceSchedule::default();
    skill_schedule.handled(OrdinarySource::Workspace);
    assert!(matches!(
        poll_ordinary(
            &mut skill_events,
            &mut skill_agent,
            &mut no_workspace,
            &mut skill,
            &skill_schedule,
            &mut skill_context,
        )
        .unwrap(),
        OrdinaryPoll::Pending
    ));
    assert_eq!(skill_state.readiness_polls.load(Ordering::Relaxed), 2);
    skill_state.wake_registered();
    assert_eq!(skill_wake.0.load(Ordering::Relaxed), 1);
}

// semantic Pending 뒤에도 source가 계속 Ready면 owner는 안전하다고 거짓으로 가정해
// park하지 않고 같은 cursor에서 selection을 다시 시작한다.
#[test]
fn persistently_ready_pending_source_requests_reselection_instead_of_waiting() {
    let mut events = UnixEventReader::new(PendingEvents::uncounted(), NeverTerminate);
    let mut agent = AlwaysReadyPendingAgent;
    let mut workspace = None;
    let mut skill = None;
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert!(matches!(
        poll_ordinary(
            &mut events,
            &mut agent,
            &mut workspace,
            &mut skill,
            &SourceSchedule::default(),
            &mut context,
        )
        .unwrap(),
        OrdinaryPoll::Reselect
    ));
}

// scheduler와 application 경계는 terminal failure와 agent closure를 오류로 유지하고,
// workspace·skill disconnect를 visible failure로 적용한 뒤 해당 연결을 제거한다.
#[test]
fn source_failures_remain_observable_when_applied() {
    let mut events = UnixEventReader::new(FailingEvents, NeverTerminate);
    let mut agent = PendingAgent::uncounted();
    let mut workspace = None;
    let mut skill = None;
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    assert!(matches!(
        poll_ordinary(
            &mut events,
            &mut agent,
            &mut workspace,
            &mut skill,
            &SourceSchedule::default(),
            &mut context,
        ),
        Err(LoopError::Input(error)) if error.contains("terminal disconnected")
    ));

    let mut agent_state = TuiState::new();
    assert!(matches!(
        apply_agent_poll(&mut agent_state, AgentPoll::Closed),
        Err(LoopError::Agent(error)) if error.contains("closed unexpectedly")
    ));

    let polls = Arc::new(AtomicUsize::new(0));
    let mut workspace_state = TuiState::new();
    workspace_state.enable_workspace_references();
    assert!(matches!(
        workspace_state
            .handle(InputEvent::Paste("@src".to_owned()), Duration::ZERO)
            .unwrap(),
        StateEffect::WorkspaceSearch(_)
    ));
    let mut workspace: Option<Box<dyn WorkspaceReferenceConnection>> =
        Some(Box::new(PendingReferences {
            readiness_polls: Arc::clone(&polls),
        }));
    assert!(apply_workspace_poll(
        &mut workspace_state,
        &mut workspace,
        Err("workspace disconnected".to_owned()),
    ));
    assert!(workspace.is_none());

    let mut skill_state = TuiState::new();
    skill_state.enable_skill_references();
    assert!(matches!(
        skill_state
            .handle(InputEvent::Paste("$skill".to_owned()), Duration::ZERO)
            .unwrap(),
        StateEffect::SkillSearch(_)
    ));
    let mut skill: Option<Box<dyn SkillReferenceConnection>> = Some(Box::new(PendingReferences {
        readiness_polls: polls,
    }));
    assert!(apply_skill_poll(
        &mut skill_state,
        &mut skill,
        Err("skill disconnected".to_owned()),
    ));
    assert!(skill.is_none());
}
