use super::*;

// presenter가 완전한 publication receipt와 함께 recovered flush를 보고하면 controller는
// 이를 버리지 않고 retained TuiSession의 bounded environmental evidence로 전파한다.
// 이후 host는 byte replay 없이 어떤 correction이 실제 사용됐는지 관찰할 수 있다.
#[test]
fn recovered_publication_receipt_is_retained_as_session_evidence() {
    let mut retained = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
    retained
        .parts_mut()
        .state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("recovered publication"),
            },
        ))
        .unwrap();
    let mut agent = SimpleAgent::default();
    let mut backend = Backend::default();
    let mut terminal = enter_screen(&mut backend, ScreenMode::Inline).unwrap();
    let render_count = Rc::new(Cell::new(0));
    let mut presenter = Presenter {
        render_count: Rc::clone(&render_count),
        recovery: Some(InlineRecovery::FlushRetry),
        ..Presenter::default()
    };
    let mut reader = UnixEventReader::new(
        Events::new([], Rc::new(Cell::new(0)), Rc::new(Cell::new(0))),
        StopAfter {
            counter: render_count,
            threshold: 1,
        },
    );

    assert!(matches!(
        drive(
            &mut terminal,
            &mut presenter,
            &mut reader,
            &mut retained,
            &mut agent,
            GenerationStart::new(Size::new(16, 6), Instant::now()),
            &mut || Ok::<Size, Infallible>(Size::new(16, 6)),
        )
        .unwrap(),
        LoopExit::Termination
    ));
    terminal.close().unwrap();

    let evidence = retained.publication_recovery_evidence();
    assert_eq!(evidence.flush_retries(), 1);
    assert_eq!(
        evidence.last(),
        Some(crate::PublicationRecoveryKind::FlushRetry)
    );
}

// persistent write가 flush된 직후 queued resize와 새 terminal size가 관찰되면 해당 Chat
// prefix는 한 번 acknowledge하되 old geometry의 live frame은 commit하지 않는다. 다음
// frame은 published prefix 없이 새 geometry에서 fresh anchor로 준비된다.
#[test]
fn post_flush_resize_acknowledges_publication_but_rejects_stale_live_geometry() {
    let mut retained = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
    retained
        .parts_mut()
        .state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("published once"),
            },
        ))
        .unwrap();
    let mut agent = SimpleAgent::default();
    let mut backend = Backend::default();
    let mut terminal = enter_screen(&mut backend, ScreenMode::Inline).unwrap();
    let render_count = Rc::new(Cell::new(0));
    let mut presenter = Presenter {
        render_count: Rc::clone(&render_count),
        ..Presenter::default()
    };
    let mut reader = UnixEventReader::new(
        Events::new(
            [Event::Resize(20, 8)],
            Rc::new(Cell::new(0)),
            Rc::new(Cell::new(0)),
        ),
        StopAfter {
            counter: render_count,
            threshold: 2,
        },
    );

    assert!(matches!(
        drive(
            &mut terminal,
            &mut presenter,
            &mut reader,
            &mut retained,
            &mut agent,
            GenerationStart::new(Size::new(16, 6), Instant::now()),
            &mut || Ok::<Size, Infallible>(Size::new(20, 8)),
        )
        .unwrap(),
        LoopExit::Termination
    ));
    terminal.close().unwrap();

    assert_eq!(presenter.publications.len(), 1);
    assert_eq!(presenter.previous_on_render, [false, false]);
    assert_eq!(presenter.invalidations, 1);
    assert_eq!(presenter.frames[0].size(), Size::new(16, 6));
    assert_eq!(presenter.frames[1].size(), Size::new(20, 8));
    assert_eq!(retained.session_output().unwrap(), None);
}

// flush 뒤 resize 알림이 원래 크기와 같은 값을 보고해도 geometry epoch가 전진했으므로
// 준비된 live frame은 폐기한다. 다음 frame은 published prefix를 반복하지 않고 fresh anchor로
// 다시 그려, 크기가 우연히 같다는 이유로 오래된 cursor 좌표를 승인하지 않는다.
#[test]
fn same_size_post_flush_resize_still_rejects_the_prepared_live_frame() {
    let mut retained = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
    retained
        .parts_mut()
        .state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("published once"),
            },
        ))
        .unwrap();
    let mut agent = SimpleAgent::default();
    let mut backend = Backend::default();
    let mut terminal = enter_screen(&mut backend, ScreenMode::Inline).unwrap();
    let render_count = Rc::new(Cell::new(0));
    let mut presenter = Presenter {
        render_count: Rc::clone(&render_count),
        ..Presenter::default()
    };
    let mut reader = UnixEventReader::new(
        Events::new(
            [Event::Resize(16, 6)],
            Rc::new(Cell::new(0)),
            Rc::new(Cell::new(0)),
        ),
        StopAfter {
            counter: render_count,
            threshold: 2,
        },
    );

    assert!(matches!(
        drive(
            &mut terminal,
            &mut presenter,
            &mut reader,
            &mut retained,
            &mut agent,
            GenerationStart::new(Size::new(16, 6), Instant::now()),
            &mut || Ok::<Size, Infallible>(Size::new(16, 6)),
        )
        .unwrap(),
        LoopExit::Termination
    ));
    terminal.close().unwrap();

    assert_eq!(presenter.publications.len(), 1);
    assert_eq!(presenter.previous_on_render, [false, false]);
    assert_eq!(presenter.invalidations, 1);
    assert_eq!(retained.session_output().unwrap(), None);
}

// persistent prefix를 flush한 뒤 terminal 크기 표본을 읽지 못해도 이미 전송된 의미 행은
// acknowledge한다. 오류를 반환하며 live anchor는 폐기하므로 재진입 시 prefix를 중복 출력하지
// 않고, 실패한 geometry의 cursor 좌표도 재사용하지 않는다.
#[test]
fn post_flush_geometry_failure_keeps_the_persistent_acknowledgement() {
    let mut retained = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
    retained
        .parts_mut()
        .state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("published before sample failure"),
            },
        ))
        .unwrap();
    let mut agent = SimpleAgent::default();
    let mut backend = Backend::default();
    let mut terminal = enter_screen(&mut backend, ScreenMode::Inline).unwrap();
    let mut presenter = Presenter::default();
    let mut reader = UnixEventReader::new(
        Events::new([], Rc::new(Cell::new(0)), Rc::new(Cell::new(0))),
        StopAfter {
            counter: Rc::new(Cell::new(0)),
            threshold: usize::MAX,
        },
    );

    let error = match drive(
        &mut terminal,
        &mut presenter,
        &mut reader,
        &mut retained,
        &mut agent,
        GenerationStart::new(Size::new(16, 6), Instant::now()),
        &mut || Err::<Size, _>("sample unavailable"),
    ) {
        Ok(_) => panic!("the geometry sample failure must stop the live generation"),
        Err(error) => error,
    };
    terminal.close().unwrap();

    assert!(error.detail().contains("sample unavailable"));
    assert_eq!(presenter.publications.len(), 1);
    assert_eq!(presenter.previous_on_render, [false]);
    assert_eq!(presenter.invalidations, 1);
    assert_eq!(retained.session_output().unwrap(), None);
}
