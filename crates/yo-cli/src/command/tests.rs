use super::*;

// 계정 용량 조회는 Session Usage와 다른 최상위 명령이며, 초기 live 구현은 외부 관측을
// 사용자가 명시한 `--refresh` 한 번으로만 시작하고 Codex source를 그대로 보존합니다.
#[test]
fn account_capacity_requires_an_explicit_refresh() {
    assert_eq!(
        parse(["account".into(), "codex".into(), "--refresh".into()]).unwrap(),
        Command::Account(AccountCommand {
            source: "codex".to_owned(),
            refresh: true,
        })
    );
    assert!(parse(["account".into(), "codex".into()]).is_err());
    assert!(parse(["account".into(), "--refresh".into()]).is_err());
}

// 인자가 없으면 기존 제품 진입점인 live Inline/Rich 실행으로 남아 `session` 기능 추가가
// 평범한 `yo`의 backend 시작 동작을 바꾸지 않는다.
#[test]
fn no_argument_keeps_the_live_defaults() {
    assert_eq!(
        parse([]).unwrap(),
        Command::Live(LiveOptions {
            mode: PresentationMode::Inline,
            glyph_profile: GlyphProfile::Rich,
            selection: LiveSelection::New,
            model: None,
            no_tools: false,
            sandbox: None,
        })
    );
}

// `-p`와 `--print`는 같은 비대화형 새 Session을 선택하고 positional prompt, model,
// creation-only no-tools를 서로 독립된 값으로 보존합니다.
#[test]
fn print_mode_has_equivalent_short_and_long_forms() {
    let expected = Command::Print(PrintOptions {
        prompt: Some("explain this".to_owned()),
        selection: LiveSelection::New,
        model: Some("qwencloud:default:qwen3.8-max".to_owned()),
        no_tools: true,
        sandbox: None,
    });

    assert_eq!(
        parse([
            "-p".into(),
            "--model".into(),
            "qwencloud:default:qwen3.8-max".into(),
            "--no-tools".into(),
            "explain this".into(),
        ])
        .unwrap(),
        expected
    );
    assert_eq!(
        parse([
            "--print".into(),
            "--model".into(),
            "qwencloud:default:qwen3.8-max".into(),
            "--no-tools".into(),
            "explain this".into(),
        ])
        .unwrap(),
        expected
    );
}

// print 입력은 stdin만 사용하는 호출을 위해 positional prompt를 생략할 수 있지만,
// prompt만 단독으로 주거나 두 개를 주면 대화형 호출로 추측하지 않고 문법 오류가 됩니다.
#[test]
fn print_prompt_is_optional_once_print_mode_is_selected() {
    assert_eq!(
        parse(["-p".into()]).unwrap(),
        Command::Print(PrintOptions {
            prompt: None,
            selection: LiveSelection::New,
            model: None,
            no_tools: false,
            sandbox: None,
        })
    );
    assert!(parse(["prompt without print".into()]).is_err());
    assert!(parse(["-p".into(), "first".into(), "second".into()]).is_err());
    assert_eq!(
        parse(["-p".into(), "--".into(), "--literal prompt".into()]).unwrap(),
        Command::Print(PrintOptions {
            prompt: Some("--literal prompt".to_owned()),
            selection: LiveSelection::New,
            model: None,
            no_tools: false,
            sandbox: None,
        })
    );
}

// print resume은 exact Session ID만 보존하고 terminal, newest-Session 추론,
// binding 교체, creation-only tool restriction을 모두 문법 단계에서 거절합니다.
#[test]
fn print_resume_is_exact_and_rejects_incompatible_startup_options() {
    let id = "01890f00-0000-7000-8000-000000000001";
    for arguments in [
        vec!["-p", "--inline"],
        vec!["-p", "--fullscreen"],
        vec!["-p", "--ascii"],
        vec!["-p", "--continue"],
        vec!["-p", "--resume", id, "--no-tools"],
        vec!["-p", "--resume", id, "--model", "kimi:default:k3"],
    ] {
        assert!(parse(arguments.into_iter().map(Into::into)).is_err());
    }

    assert_eq!(
        parse([
            "-p".into(),
            "--resume".into(),
            id.into(),
            "follow up".into()
        ])
        .unwrap(),
        Command::Print(PrintOptions {
            prompt: Some("follow up".to_owned()),
            selection: LiveSelection::Resume(id.parse().unwrap()),
            model: None,
            no_tools: false,
            sandbox: None,
        })
    );
}

// print mode와 top-level subcommand를 한 호출에 섞으면 subcommand나 prompt 중 하나를
// 조용히 우선하지 않고, 사용자가 `--` 뒤의 명시적 literal prompt로 고치게 합니다.
#[test]
fn print_rejects_top_level_subcommands_without_literal_separator() {
    for subcommand in ["connect", "disconnect", "default", "session", "usage"] {
        let error = parse(["-p".into(), subcommand.into()]).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
        assert!(
            error
                .to_string()
                .contains("use `--` before a literal prompt")
        );
    }

    assert_eq!(
        parse(["-p".into(), "--".into(), "session".into()]).unwrap(),
        Command::Print(PrintOptions {
            prompt: Some("session".to_owned()),
            selection: LiveSelection::New,
            model: None,
            no_tools: false,
            sandbox: None,
        })
    );

    assert_eq!(
        parse([
            "-p".into(),
            "--model".into(),
            "session".into(),
            "question".into(),
        ])
        .unwrap(),
        Command::Print(PrintOptions {
            prompt: Some("question".to_owned()),
            selection: LiveSelection::New,
            model: Some("session".to_owned()),
            no_tools: false,
            sandbox: None,
        })
    );
}

// `--` 뒤의 print 표기는 literal subcommand argument이므로 앞서 나온 subcommand와
// print mode의 충돌로 오분류하지 않고, 이후 clap 문법만 해당 입력을 판단합니다.
#[test]
fn literal_print_flags_after_separator_do_not_trigger_print_conflict() {
    for arguments in [["session", "--", "-p"], ["usage", "--", "--print"]] {
        let error = parse(arguments.map(Into::into)).unwrap_err();
        assert!(!error.to_string().contains("cannot be combined with"));
    }
}

// `--no-tools`는 새 live Session의 명시적 restriction으로 전달되고, 저장된 replay
// contract를 다시 쓰지 않도록 resume·continue와의 조합은 문법 단계에서 거절합니다.
#[test]
fn live_no_tools_is_creation_only() {
    let Command::Live(options) = parse(["--no-tools".into()]).unwrap() else {
        panic!("--no-tools remains a live startup option");
    };
    assert!(options.no_tools);

    let id = "01890f00-0000-7000-8000-000000000001";
    assert!(parse(["--no-tools".into(), "--resume".into(), id.into()]).is_err());
    assert!(parse(["--no-tools".into(), "--continue".into()]).is_err());
}

// `--sandbox read-only`는 비대화형 delegated host 리뷰를 명시하는 값으로 보존되고,
// 평범한 live 실행이나 native tool restriction으로 잘못 해석되지 않습니다.
#[test]
fn read_only_sandbox_is_a_print_only_creation_profile() {
    assert_eq!(
        parse([
            "-p".into(),
            "--model".into(),
            "host:codex".into(),
            "--sandbox".into(),
            "read-only".into(),
            "review".into(),
        ])
        .unwrap(),
        Command::Print(PrintOptions {
            prompt: Some("review".to_owned()),
            selection: LiveSelection::New,
            model: Some("host:codex".to_owned()),
            no_tools: false,
            sandbox: Some(SandboxMode::ReadOnly),
        })
    );

    let id = "01890f00-0000-7000-8000-000000000001";
    for arguments in [
        vec!["--sandbox", "read-only"],
        vec!["-p", "--sandbox", "read-only", "--no-tools"],
        vec!["-p", "--sandbox", "read-only", "--resume", id],
        vec!["-p", "--sandbox", "writable", "review"],
    ] {
        assert!(parse(arguments.into_iter().map(Into::into)).is_err());
    }
}

// `--model`은 새 Session과 resume 양쪽에서 같은 명시적 model reference로 보존되고,
// Provider·Account 변경 option은 노출하지 않으며 중복 값이나 빠진 값은 사용법 오류가 된다.
#[test]
fn live_model_override_is_explicit_and_single() {
    let Command::Live(options) =
        parse(["--model".into(), "qwencloud:default:qwen3.8-max".into()]).unwrap()
    else {
        panic!("--model remains a live startup option");
    };
    assert_eq!(
        options.model.as_deref(),
        Some("qwencloud:default:qwen3.8-max")
    );

    assert!(parse(["--provider".into(), "qwencloud".into()]).is_err());
    assert!(parse(["--account".into(), "default".into()]).is_err());
    assert!(parse(["--model".into()]).is_err());
    assert!(
        parse([
            "--model".into(),
            "first".into(),
            "--model".into(),
            "second".into(),
        ])
        .is_err()
    );
}

// 하이픈으로 시작하는 model reference도 CLI option으로 재해석하지 않고 Yo의 model
// resolver에 그대로 전달해, model 유효성 및 실패 의미의 소유권을 제품 경계에 남깁니다.
#[test]
fn option_like_model_reference_reaches_the_model_resolver_unchanged() {
    let Command::Live(options) = parse(["--model".into(), "-vendor-model".into()]).unwrap() else {
        panic!("--model remains a live startup option");
    };

    assert_eq!(options.model.as_deref(), Some("-vendor-model"));
}

// `default TARGET`과 `default --unset`은 정확히 하나의 새 기본값 의도를 만들고, 둘을
// 함께 주거나 아무것도 주지 않으면 어떤 상태를 게시할지 추측하지 않고 거절합니다.
#[test]
fn default_command_requires_exactly_one_set_or_clear_intent() {
    assert_eq!(
        parse(["default".into(), "host:codex".into()]).unwrap(),
        Command::Default(DefaultCommand {
            target: Some("host:codex".to_owned())
        })
    );
    assert_eq!(
        parse(["default".into(), "--unset".into()]).unwrap(),
        Command::Default(DefaultCommand { target: None })
    );
    assert!(parse(["default".into()]).is_err());
    assert!(parse(["default".into(), "host:codex".into(), "--unset".into(),]).is_err());
}

// connect 문법은 하나의 exact target을 필수로 보존하고 값 없는 호출이나 복수 target을
// onboarding 선택으로 오해하지 않아 아직 준비되지 않은 결정을 만들지 않습니다.
#[test]
fn connect_command_requires_one_exact_target() {
    assert_eq!(
        parse(["connect".into(), "host:codex".into()]).unwrap(),
        Command::Connect(ConnectCommand {
            from: None,
            target: "host:codex".to_owned(),
            verbose: false,
            credential_file: None,
            yes: false,
        })
    );
    assert_eq!(
        parse(["connect".into(), "host:codex".into(), "-v".into()]).unwrap(),
        Command::Connect(ConnectCommand {
            from: None,
            target: "host:codex".to_owned(),
            verbose: true,
            credential_file: None,
            yes: false,
        })
    );
    assert!(parse(["connect".into()]).is_err());
    assert!(parse(["connect".into(), "host:codex".into(), "second".into(),]).is_err());
    assert_eq!(
        parse(["connect".into(), "--from".into(), "/tmp/models.yaml".into(),]).unwrap(),
        Command::Connect(ConnectCommand {
            target: String::new(),
            from: Some("/tmp/models.yaml".into()),
            verbose: false,
            credential_file: None,
            yes: false,
        })
    );
    assert_eq!(
        parse(["connect".into(), "--from".into(), "-".into(),]).unwrap(),
        Command::Connect(ConnectCommand {
            target: String::new(),
            from: Some("-".into()),
            verbose: false,
            credential_file: None,
            yes: false,
        })
    );
    assert!(
        parse([
            "connect".into(),
            "host:codex".into(),
            "--from".into(),
            "/tmp/models.yaml".into(),
        ])
        .is_err()
    );
}

// 비대화형 external connect는 credential path와 exact-plan 승인 둘을 함께 요구하고,
// verbose interactive confirmation과 조합하거나 어느 한쪽만 주는 호출을 문법에서 거절합니다.
#[test]
fn non_interactive_connect_requires_the_closed_file_and_yes_pair() {
    assert_eq!(
        parse([
            "connect".into(),
            "vendor:team:model".into(),
            "--credential-file".into(),
            "/run/secrets/vendor".into(),
            "--yes".into(),
        ])
        .unwrap(),
        Command::Connect(ConnectCommand {
            from: None,
            target: "vendor:team:model".to_owned(),
            verbose: false,
            credential_file: Some("/run/secrets/vendor".into()),
            yes: true,
        })
    );
    assert!(
        parse([
            "connect".into(),
            "vendor:team:model".into(),
            "--credential-file".into(),
            "/run/secrets/vendor".into(),
        ])
        .is_err()
    );
    assert!(parse(["connect".into(), "vendor:team:model".into(), "--yes".into(),]).is_err());
    assert!(
        parse([
            "connect".into(),
            "vendor:team:model".into(),
            "--credential-file".into(),
            "/run/secrets/vendor".into(),
            "--yes".into(),
            "--verbose".into(),
        ])
        .is_err()
    );

    let help = parse(["connect".into(), "--help".into()]).unwrap_err();
    let rendered = help.to_string();
    assert!(rendered.contains("--credential-file <PATH>"));
    assert!(rendered.contains("--yes"));
}

// disconnect는 인자 없는 대화형 선택을 허용하되, 자동 실행은 exact Provider와 Account와
// --yes를 모두 요구해 범위가 빠진 승인을 비대화형 삭제 권한으로 해석하지 않습니다.
#[test]
fn disconnect_separates_interactive_selection_from_exact_automatic_authorization() {
    assert_eq!(
        parse(["disconnect".into()]).unwrap(),
        Command::Disconnect(DisconnectCommand {
            provider: None,
            account: None,
            yes: false,
            verbose: false,
        })
    );
    assert_eq!(
        parse([
            "disconnect".into(),
            "vendor".into(),
            "--account".into(),
            "team".into(),
            "--yes".into(),
        ])
        .unwrap(),
        Command::Disconnect(DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: true,
            verbose: false,
        })
    );
    assert_eq!(
        parse(["disconnect".into(), "-v".into()]).unwrap(),
        Command::Disconnect(DisconnectCommand {
            provider: None,
            account: None,
            yes: false,
            verbose: true,
        })
    );
    assert!(parse(["disconnect".into(), "--yes".into()]).is_err());
    assert!(parse(["disconnect".into(), "vendor".into(), "--yes".into(),]).is_err());
    assert!(parse(["disconnect".into(), "--account".into(), "team".into(),]).is_err());
}

// 명시한 UUID 재개와 현재 작업공간의 최근 세션 재개는 새 Session 시작과 구분되고,
// 동시에 지정하면 어느 쪽도 임의로 우선하지 않는다.
#[test]
fn live_continuation_options_are_explicit_and_mutually_exclusive() {
    let id = "01890f00-0000-7000-8000-000000000001";
    let Command::Live(resume) = parse(["--resume".into(), id.into()]).unwrap() else {
        panic!("--resume remains a live startup option");
    };
    assert_eq!(resume.selection, LiveSelection::Resume(id.parse().unwrap()));

    let Command::Live(continuation) = parse(["--continue".into()]).unwrap() else {
        panic!("--continue remains a live startup option");
    };
    assert_eq!(continuation.selection, LiveSelection::Continue);

    let error = parse(["--continue".into(), "--resume".into(), id.into()]).unwrap_err();
    assert!(error.to_string().contains("--resume"));
}

// 목록 option은 Session ID 없이 조합할 수 있고 `--details`가 선택 집합을 바꾸는 별도
// command가 아니라 같은 목록의 metadata 확장으로 해석된다.
#[test]
fn session_list_accepts_all_and_details_in_any_order() {
    let command = parse(["session".into(), "--details".into(), "--all".into()]).unwrap();

    assert_eq!(
        command,
        Command::Session(SessionCommand {
            session_id: None,
            all: true,
            details: true,
            view: SessionView::Chat,
            glyph_profile: GlyphProfile::Rich,
            limit: None,
            content: None,
        })
    );
}

// full UUID 뒤의 Transcript view와 ASCII 선택은 저장 history를 읽는 표시 옵션으로만
// 결합되고 live presentation mode나 writer 설정으로 새지 않는다.
#[test]
fn direct_session_selects_a_read_only_projection() {
    let id = "01890f00-0000-7000-8000-000000000001";
    let command = parse([
        "session".into(),
        id.into(),
        "--view".into(),
        "transcript".into(),
        "--ascii".into(),
    ])
    .unwrap();

    let Command::Session(command) = command else {
        panic!("the session command remains distinct from live startup");
    };
    assert_eq!(command.session_id.unwrap().to_string(), id);
    assert_eq!(command.view, SessionView::Transcript);
    assert_eq!(command.glyph_profile, GlyphProfile::Ascii);
    assert_eq!(command.limit, None);
    assert_eq!(command.content, None);
}

// `request`는 같은 UUID의 전체 저장 상관 흐름을 읽는 세 번째 view로 파싱되고,
// 특정 시점을 추측하는 `--at` 선택자는 실제 사용처가 생기기 전까지 명시적으로 거부됩니다.
#[test]
fn direct_session_accepts_request_without_an_anchor_selector() {
    let id = "01890f00-0000-7000-8000-000000000001";
    let Command::Session(command) = parse([
        "session".into(),
        id.into(),
        "--view".into(),
        "request".into(),
    ])
    .unwrap() else {
        panic!("the session command remains distinct from live startup");
    };

    assert_eq!(command.view, SessionView::Request);
    let error = parse([
        "session".into(),
        id.into(),
        "--view".into(),
        "request".into(),
        "--at".into(),
        "5".into(),
    ])
    .unwrap_err();
    assert!(error.to_string().contains("unexpected argument '--at'"));
}

// Transcript의 limit와 content는 최신 semantic record 수와 payload 노출량만 정하며,
// 0 limit은 파싱 단계에서 거부되어 저장소를 읽기 전에 실패한다.
#[test]
fn transcript_accepts_bounded_content_selectors() {
    let id = "01890f00-0000-7000-8000-000000000001";
    let Command::Session(command) = parse([
        "session".into(),
        id.into(),
        "--view".into(),
        "transcript".into(),
        "--limit".into(),
        "2".into(),
        "--content".into(),
        "preview".into(),
    ])
    .unwrap() else {
        panic!("bounded Transcript remains a stored Session projection");
    };

    assert_eq!(command.limit.unwrap().get(), 2);
    assert_eq!(command.content, Some(SessionContent::Preview));
    assert!(
        parse([
            "session".into(),
            id.into(),
            "--view".into(),
            "transcript".into(),
            "--limit".into(),
            "0".into(),
        ])
        .is_err()
    );
}

// 명시한 selector는 `full`도 Transcript 전용이므로 Chat·Request·목록에서는 저장소 접근
// 전에 일관된 사용법 오류가 되고, selector를 생략한 기존 경로만 완전 출력으로 남는다.
#[test]
fn transcript_selectors_are_rejected_for_other_session_surfaces() {
    let id = "01890f00-0000-7000-8000-000000000001";
    for arguments in [
        vec!["session", id, "--content", "full"],
        vec!["session", id, "--view", "request", "--limit", "1"],
    ] {
        let error = parse(arguments.into_iter().map(Into::into)).unwrap_err();
        assert!(error.to_string().contains("only with --view transcript"));
    }
    assert!(parse(["session".into(), "--content".into(), "none".into()]).is_err());
}

// 최상위 `usage`는 정확한 Session UUID와 glyph profile만 보존하며, live startup이나
// provider 선택 option을 만들지 않고 보관된 Usage 조회 command로 분리됩니다.
#[test]
fn top_level_usage_selects_one_archived_session() {
    let id = "01890f00-0000-7000-8000-000000000001";
    let command = parse(["usage".into(), id.into(), "--ascii".into()]).unwrap();

    assert_eq!(
        command,
        Command::Usage(UsageCommand {
            session_id: id.parse().unwrap(),
            glyph_profile: GlyphProfile::Ascii,
        })
    );
    assert!(parse(["usage".into()]).is_err());
    assert!(parse(["usage".into(), "not-a-session-id".into()]).is_err());
    assert!(parse(["usage".into(), id.into(), "second".into()]).is_err());
}

// Usage는 Session view 값이 아니므로 이전 별칭은 허용하지 않고, 최상위 읽기 전용 명령만
// 한 개의 공개 문법으로 남긴다.
#[test]
fn usage_is_not_a_session_view_alias() {
    let id = "01890f00-0000-7000-8000-000000000001";
    let error = parse(["session".into(), id.into(), "--view".into(), "usage".into()]).unwrap_err();

    assert!(error.to_string().contains("invalid value 'usage'"));
}

// list 전용 `--all`과 direct read UUID를 함께 쓰면 어느 쪽 의미도 임의로 우선하지 않고
// 사용법 오류로 거부해 조회 범위와 출력 대상이 모호해지지 않는다.
#[test]
fn list_only_options_are_rejected_for_a_direct_session() {
    let error = parse([
        "session".into(),
        "01890f00-0000-7000-8000-000000000001".into(),
        "--all".into(),
    ])
    .unwrap_err();

    assert!(error.to_string().contains("cannot be used with"));
}

// `--help`는 실패가 아니라 stdout으로 전달할 성공 제어 흐름이며, 생성된 문서에서 live
// option과 저장 Session 하위 명령을 한 진입점의 서로 다른 사용 경로로 보여 줍니다.
#[test]
fn help_is_successful_generated_command_documentation() {
    let help = parse(["--help".into()]).unwrap_err();

    assert_eq!(help.kind(), clap::error::ErrorKind::DisplayHelp);
    assert_eq!(help.exit_code(), 0);
    assert!(!help.use_stderr());
    let rendered = help.to_string();
    assert!(rendered.contains("Usage: yo [OPTIONS]"));
    assert!(rendered.contains("yo <COMMAND>"));
    assert!(rendered.contains("session"));
    assert!(rendered.contains("usage"));
    assert!(rendered.contains("account"));
    assert!(rendered.contains("disconnect"));
    assert!(rendered.contains("--model <MODEL_REFERENCE>"));
    assert!(rendered.contains("--no-tools"));
    assert!(rendered.contains("-p, --print"));
    assert!(rendered.contains("[PROMPT]"));
}

// `--version`도 도움말과 같은 성공 제어 흐름으로 stdout에 전달되고, Cargo package
// version을 한 곳에서 가져와 수동 version 문자열과 실행 파일의 불일치를 만들지 않습니다.
#[test]
fn version_is_successful_generated_output() {
    let version = parse(["--version".into()]).unwrap_err();

    assert_eq!(version.kind(), clap::error::ErrorKind::DisplayVersion);
    assert_eq!(version.exit_code(), 0);
    assert!(!version.use_stderr());
    assert_eq!(
        version.to_string(),
        format!("yo {}\n", env!("CARGO_PKG_VERSION"))
    );
}

// 사용자가 option 철자를 틀리면 전체 수동 usage를 한 줄로 반복하지 않고, clap이 가장
// 가까운 실제 option과 짧은 명령별 Usage를 함께 제안해 복구 경로를 직접 보여 줍니다.
#[test]
fn misspelled_option_suggests_the_supported_spelling() {
    let error = parse(["--modle".into(), "host:codex".into()]).unwrap_err();
    let rendered = error.to_string();

    assert!(rendered.contains("unexpected argument '--modle'"));
    assert!(rendered.contains("similar argument exists: '--model'"));
    assert!(rendered.contains("Usage: yo --model <MODEL_REFERENCE>"));
}
