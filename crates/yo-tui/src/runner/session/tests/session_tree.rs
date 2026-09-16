use super::*;

// 실제 durable fork에서 부모 파일이 없더라도 placeholder는 선택에서 건너뛰고 child는
// 좁은 20/40열 panel에서도 기존 resume route로 선택합니다.
#[test]
fn session_tree_keeps_missing_parent_disabled_and_child_selectable_at_narrow_widths() {
    use std::{
        env, fs,
        num::NonZeroU64,
        path, thread,
        time::{Duration, Instant},
    };

    use yo_core::{
        AgentCommand, AgentEvent, AgentIntent, AgentSession, BackendBindingEvidence,
        BackendCommandEvidence, BackendEvent, BackendIdentity, BackendOutcomeEvidence,
        BackendRequestEvidence, BackendScriptStep, CommandAdmission, ContextPolicyChanged,
        ContextStrategy, ContinuationStrategy, HostWorkspacePath, InputSubmission,
        ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole, ReplayExecutor,
        ReplayProfile, ScriptedBackend, SessionDescriptor, SessionId, SubmissionId,
        TranscriptRecord, TurnId, TurnOutcome, TurnRef, WorkspaceHostId,
        session_repository::{
            self, LocalSessionReader, LocalSessionRepository, SessionTreeLimits,
            StoredSessionReader, read_stored_session_continuation,
        },
    };

    use crate::{
        input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
        runner::state::StateEffect,
        surface::{CellContent, Point, Size},
    };
    let root = env::temp_dir().join(format!("yo-tui-tree-{}", SessionId::new().unwrap()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let host: WorkspaceHostId = "10000000-0000-4000-8000-000000000001".parse().unwrap();
    let workspace = HostWorkspacePath::normalize_local(&root).unwrap();
    let parent_id = SessionId::new().unwrap();
    let descriptor = SessionDescriptor::for_session(parent_id, host, workspace.clone());
    let binding = |id: SessionId| {
        BackendBindingEvidence::new(
            "managed",
            "1",
            BackendIdentity::new("binding/v1", "account"),
            BackendIdentity::new("model/v1", "model"),
            BackendIdentity::new("locator/v1", id.to_string()),
            ContinuationStrategy::ExactReplay {
                executor: ReplayExecutor::LocalClient,
                replay_profile: ReplayProfile::SemanticOnly,
            },
        )
    };
    let turn = TurnRef::new(parent_id, TurnId::new(NonZeroU64::new(1).unwrap()));
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::CreateSession {
                session_id: parent_id,
            },
            evidence: BackendCommandEvidence::BindingOpened(binding(parent_id)),
        },
        BackendScriptStep::Emit(BackendEvent::ContextPolicyChanged {
            policy: ContextPolicyChanged::try_new(
                1,
                true,
                ContextStrategy::PortableSummaryV1Alpha1,
                85,
                90,
                Some(10),
                Some(65536),
            )
            .unwrap(),
        }),
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn,
                input: "question".into(),
            },
            evidence: BackendCommandEvidence::RequestAccepted(BackendRequestEvidence::new(
                "request/v1",
                BackendIdentity::new("exchange/v1", "1"),
                BackendIdentity::new("accepted/v1", "1"),
            )),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn,
            evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                "outcome/v1",
                "1",
            ))
            .with_replay(ModelReplayDelta::new(
                Some(ModelReplayContract::new("system", vec![])),
                vec![
                    ModelReplayItem::Message {
                        role: ModelReplayRole::User,
                        content: "question".into(),
                        refusal: None,
                    },
                    ModelReplayItem::Message {
                        role: ModelReplayRole::Assistant,
                        content: "answer".into(),
                        refusal: None,
                    },
                ],
            )),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut agent = AgentSession::start_cancellable_with_repository(
        backend,
        descriptor,
        LocalSessionRepository::open(&root, 1024 * 1024).unwrap(),
        || Instant::now() >= deadline,
    )
    .unwrap()
    .unwrap();
    let mut admission = agent
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            SubmissionId::new().unwrap(),
            "question".into(),
        )))
        .unwrap();
    while let CommandAdmission::Backpressured(pending) = admission {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
        admission = agent.retry(pending).unwrap();
    }
    while !agent
        .transcript_reader()
        .read_after(None)
        .entries()
        .iter()
        .any(|entry| {
            matches!(
                entry.record(),
                TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                    outcome: TurnOutcome::Completed,
                    ..
                })
            )
        })
    {
        assert!(Instant::now() < deadline);
        agent.poll().unwrap();
        thread::sleep(Duration::from_millis(1));
    }
    agent.shutdown().unwrap();
    drop(agent);
    let reader = LocalSessionReader::open(&root).unwrap();
    let parent = read_stored_session_continuation(&reader, parent_id).unwrap();
    let child_id = SessionId::new().unwrap();
    let child = parent
        .prepare_exact_fork(
            SessionDescriptor::for_session(child_id, host, workspace.clone()),
            binding(child_id),
        )
        .unwrap();
    let backend = ScriptedBackend::new([
        BackendScriptStep::Resume {
            target: Box::new(child.target().clone()),
            evidence: binding(child_id),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut agent = AgentSession::start_cancellable_with_continuation(
        backend,
        child,
        LocalSessionRepository::open(&root, 1024 * 1024).unwrap(),
        || Instant::now() >= deadline,
    )
    .unwrap()
    .unwrap();
    agent.shutdown().unwrap();
    drop(agent);
    fs::remove_file(root.join(format!("{parent_id}.jsonl"))).unwrap();
    let tree = reader
        .read_tree(host, &workspace, SessionTreeLimits::default())
        .unwrap();
    assert_eq!(tree.nodes().len(), 2);
    for width in [20, 40] {
        let mut tui = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
        tui.state
            .observe_durability(yo_core::JournalDurability::Durable {
                journal_sequence: None,
                repository_sequence: session_repository::RepositorySequence::new(1),
            })
            .unwrap();
        tui.show_session_tree(&tree, parent_id).unwrap();
        let frame = tui
            .state
            .prepare_frame(Size::new(width, 24), &tui.appearance.pin())
            .unwrap();
        assert!(frame.overlay_presented);
        let visible = (0..24)
            .flat_map(|y| (0..width).map(move |x| Point::new(x, y)))
            .filter_map(|point| match frame.surface.cell(point).unwrap().content() {
                CellContent::Grapheme { text, .. } => Some(text.to_string()),
                _ => None,
            })
            .collect::<String>();
        let compact = visible
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>()
            .to_lowercase();
        assert!(
            compact.contains(&child_id.to_string()),
            "full child UUID missing at width {width}"
        );
        assert!(
            compact.contains(&parent_id.to_string()),
            "full parent UUID missing at width {width}"
        );
        assert!(
            compact.contains("ancestormissing"),
            "missing ancestor status clipped at width {width}"
        );
        tui.state.commit_frame(&frame);
        let result = tui
            .state
            .handle(
                InputEvent::Key(KeyEvent {
                    code: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                    action: KeyAction::Press,
                    state: KeyState::NONE,
                }),
                Duration::ZERO,
            )
            .unwrap();
        assert_eq!(result, StateEffect::Exit);
        assert_eq!(tui.take_resume_session_request(), Some(Some(child_id)));
        let mut readonly = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
        readonly.show_session_tree(&tree, child_id).unwrap();
        let mut viewed = String::new();
        for _ in 0..2 {
            let frame = readonly
                .state
                .prepare_frame(Size::new(width, 24), &readonly.appearance.pin())
                .unwrap();
            assert!(frame.overlay_presented);
            for y in 0..24 {
                for x in 0..width {
                    if let CellContent::Grapheme { text, .. } =
                        frame.surface.cell(Point::new(x, y)).unwrap().content()
                    {
                        viewed.push_str(text);
                    }
                }
            }
            readonly.state.commit_frame(&frame);
            readonly
                .state
                .handle(
                    InputEvent::Key(KeyEvent {
                        code: KeyCode::Down,
                        modifiers: KeyModifiers::NONE,
                        action: KeyAction::Press,
                        state: KeyState::NONE,
                    }),
                    Duration::ZERO,
                )
                .unwrap();
        }
        let compact = viewed
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>()
            .to_lowercase();
        assert!(compact.contains(&parent_id.to_string()));
        assert!(compact.contains(&child_id.to_string()));
        assert!(compact.contains("currentsession"));
        assert!(compact.contains("ancestorunavailable"));
        readonly
            .state
            .handle(
                InputEvent::Key(KeyEvent {
                    code: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                    action: KeyAction::Press,
                    state: KeyState::NONE,
                }),
                Duration::ZERO,
            )
            .unwrap();
        assert_eq!(readonly.take_resume_session_request(), None);
    }
}
