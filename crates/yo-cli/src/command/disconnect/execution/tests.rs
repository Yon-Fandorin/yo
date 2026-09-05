use std::{
    fs,
    io::{ErrorKind, Read, Write},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime},
};

use nix::{
    fcntl::{FcntlArg, OFlag, fcntl},
    pty::openpty,
};
use yo_core::{
    AccountId, ApiCredential, CompleteModelBinding, ConnectionAccount, ConnectionCatalogSeed,
    LocalConnectionRepository, LocalCredentialRepository, ModelId, ProviderId, VersionedProfileId,
};

use super::*;

struct FakeInput {
    selected: Option<String>,
    confirmed: bool,
    selections: Vec<Vec<String>>,
    summaries: Vec<String>,
}

impl ExternalDisconnectInput for FakeInput {
    fn select_target(&mut self, choices: &[String]) -> Result<String, AppError> {
        self.selections.push(choices.to_vec());
        self.selected
            .clone()
            .ok_or_else(|| AppError::message("no fake selection"))
    }

    fn confirm(
        &mut self,
        preview: &dyn crate::interaction::connection::ConfirmationView,
    ) -> Result<bool, AppError> {
        self.summaries.push(
            preview
                .render_styled(
                    crate::interaction::connection::default_width(),
                    crate::interaction::PresentationStyle::Plain,
                )
                .unwrap(),
        );
        Ok(self.confirmed)
    }
}

// --yes는 exact Provider/Account 아래 stored target이 하나일 때만 TTY 없이 실행하고,
// public binding과 matching preference를 지운 뒤 마지막 dependent credential도 제거합니다.
#[test]
fn automatic_unique_disconnect_removes_public_then_credential_without_prompt() {
    let fixture = Fixture::new("automatic");
    let config_path = fixture.config_path("session: {}\n");
    fixture.seed_stored(&["alpha"], Some("alpha"));
    fixture.seed_credential();
    let mut input = FakeInput {
        selected: None,
        confirmed: false,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    let output = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: true,
            verbose: false,
        },
        &mut input,
    )
    .unwrap();

    assert_eq!(
        output,
        "✓ Disconnected\n\n  Model    vendor:team:alpha\n  API key  Removed\n  Default  unset\n"
    );
    assert!(input.selections.is_empty());
    assert!(input.summaries.is_empty());
    assert!(fixture.connections().capture().unwrap().models().is_empty());
    assert!(
        fixture
            .credentials()
            .capture()
            .unwrap()
            .resolve(&provider(), &account())
            .is_none()
    );
    assert!(!fixture.root.join("connection-operation.yaml").exists());
}

// 성공 요약은 내부 exact ModelId를 바꾸지 않으면서 bidi·zero-width·일반 Unicode를
// reversible printable-ASCII byte escape로 표시해 terminal 방향과 가시성을 보호합니다.
#[test]
fn disconnect_success_escapes_non_ascii_model_identity() {
    let fixture = Fixture::new("escaped-success");
    let config_path = fixture.config_path("session: {}\n");
    fixture.seed_stored(&["alpha\u{202e}\u{200b}한"], None);
    fixture.seed_credential();
    let mut input = FakeInput {
        selected: None,
        confirmed: false,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    let output = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: true,
            verbose: false,
        },
        &mut input,
    )
    .unwrap();

    assert!(
        output.contains("Model    vendor:team:alpha\\xE2\\x80\\xAE\\xE2\\x80\\x8B\\xED\\x95\\x9C")
    );
    assert!(!output.contains('\u{202e}'));
    assert!(!output.contains('\u{200b}'));
    assert!(!output.contains('한'));
}

// 여러 stored target 중 하나를 대화형으로 고르면 preview가 exact removed profile,
// preference 전이, 남는 distinct binding, credential preserve와 resume risk를 모두 표시합니다.
#[test]
fn interactive_preview_selects_one_and_discloses_the_complete_preserve_plan() {
    let fixture = Fixture::new("interactive");
    let config_path = fixture.config_path("session: {}\n");
    fixture.seed_stored(&["alpha", "beta"], Some("alpha"));
    fixture.seed_credential();
    let mut input = FakeInput {
        selected: Some("vendor:team:beta".to_owned()),
        confirmed: false,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    let output = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: None,
            account: None,
            yes: false,
            verbose: true,
        },
        &mut input,
    )
    .unwrap();

    assert_eq!(output, "Disconnect cancelled; nothing changed.\n");
    assert_eq!(
        input.selections,
        [vec![
            "vendor:team:alpha".to_owned(),
            "vendor:team:beta".to_owned()
        ]]
    );
    let summary = &input.summaries[0];
    assert!(summary.contains("Connection being removed"));
    assert!(summary.contains("vendor:team:beta"));
    assert!(summary.contains("Keep vendor:team:alpha"));
    assert!(summary.contains("Keep — still used by alpha"));
    assert!(summary.contains("vendor:team:alpha"));
    assert!(summary.contains("Unavailable until this exact model is restored"));
    assert_eq!(fixture.connections().capture().unwrap().models().len(), 2);
    assert!(
        fixture
            .credentials()
            .capture()
            .unwrap()
            .resolve(&provider(), &account())
            .is_some()
    );

    let mut compact_input = FakeInput {
        selected: Some("vendor:team:beta".to_owned()),
        confirmed: false,
        selections: Vec::new(),
        summaries: Vec::new(),
    };
    execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: None,
            account: None,
            yes: false,
            verbose: false,
        },
        &mut compact_input,
    )
    .unwrap();
    let compact = &compact_input.summaries[0];
    assert!(compact.contains("Keep — still used by alpha"));
    assert!(!compact.contains("Connection being removed"));
    assert!(!compact.contains("Still available for this account"));
}

// Compact disconnect의 credential 보존 목록도 쉼표·공백이 든 합법적 Model ID를
// 따옴표로 구분해, 여러 평범한 ID를 이어 쓴 목록과 혼동되지 않게 합니다.
#[test]
fn compact_preview_quotes_a_delimiter_bearing_remaining_model() {
    let fixture = Fixture::new("quoted-model-list");
    let config_path = fixture.config_path("session: {}\n");
    fixture.seed_stored(&["alpha, beta", "gamma"], None);
    fixture.seed_credential();
    let mut input = FakeInput {
        selected: Some("vendor:team:gamma".to_owned()),
        confirmed: false,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: None,
            account: None,
            yes: false,
            verbose: false,
        },
        &mut input,
    )
    .unwrap();

    assert!(input.summaries[0].contains("Keep — still used by \"alpha, beta\""));
    assert!(!input.summaries[0].contains("Keep — still used by alpha, beta"));
}

// 저장 preference를 제거해도 더 낮은 startup source가 있으면 preview는 막연한 재설정
// 경고 대신 실제 prospective resolver가 선택할 exact target을 보여 줍니다.
#[test]
fn preview_resolves_the_exact_lower_priority_startup_target() {
    let fixture = Fixture::new("startup-fallback");
    fixture.seed_stored(&["alpha"], Some("alpha"));
    let snapshot = fixture.connections().capture().unwrap();
    let selection = ModelSelection::new(provider(), account(), ModelId::new("alpha").unwrap());
    let policies = [
        StartupPolicy::new(true, None, Some(StartupTarget::host_codex())).unwrap(),
        StartupPolicy::new(false, Some(StartupTarget::host_codex()), None).unwrap(),
    ];

    for policy in policies {
        let plan = ExternalDisconnectPlan::prepare(&snapshot, &selection, &policy, false).unwrap();
        let preview = plan
            .preview
            .render(crate::interaction::connection::default_width())
            .unwrap();

        assert!(preview.contains("✓ New sessions\n  Use host:codex"));
        assert!(!preview.contains("No startup target remains"));
    }
}

// --yes 범위가 둘 이상의 stored target이면 선택 오류가 나며, 기존 binding·preference·
// account/catalog·credential의 값과 revision 및 operation journal 부재를 모두 보존합니다.
#[test]
fn automatic_ambiguity_preserves_every_repository_state_before_disconnect_intent() {
    let fixture = Fixture::new("selection-errors");
    let config_path = fixture.config_path("session: {}\n");
    fixture.seed_stored(&["alpha", "beta"], Some("alpha"));
    fixture.seed_credential();
    let before_connections = fixture.connections().capture().unwrap();
    let before_credentials = fixture.credentials().capture().unwrap();
    let mut input = FakeInput {
        selected: None,
        confirmed: true,
        selections: Vec::new(),
        summaries: Vec::new(),
    };
    let error = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: true,
            verbose: false,
        },
        &mut input,
    )
    .unwrap_err();

    assert!(error.to_string().contains("--yes never guesses"));
    assert!(input.selections.is_empty());
    assert!(input.summaries.is_empty());

    let after_connections = fixture.connections().capture().unwrap();
    assert_eq!(after_connections.revision(), before_connections.revision());
    assert_eq!(after_connections.models(), before_connections.models());
    assert_eq!(
        after_connections.preference(),
        before_connections.preference()
    );
    assert_eq!(after_connections.accounts(), before_connections.accounts());
    assert_eq!(
        after_connections.catalog_seeds(),
        before_connections.catalog_seeds()
    );

    let after_credentials = fixture.credentials().capture().unwrap();
    assert_eq!(after_credentials.revision(), before_credentials.revision());
    assert_eq!(
        after_credentials.resolve(&provider(), &account()),
        before_credentials.resolve(&provider(), &account())
    );
    assert!(!fixture.root.join("connection-operation.yaml").exists());
}

// stored target이 하나도 없으면 선택 오류가 나고, 비어 있던 connection·credential·
// operation journal 경로를 만들지 않아 pre-intent 실패가 저장소를 생성하지 않음을 보입니다.
#[test]
fn zero_candidate_selection_error_leaves_repository_paths_absent() {
    let fixture = Fixture::new("zero-candidate-selection-error");
    let config_path = fixture.config_path("session: {}\n");
    let connection_path = fixture.root.join("connections.yaml");
    let credential_path = fixture.root.join("credentials.yaml");
    let operation_path = fixture.root.join("connection-operation.yaml");
    assert!(!connection_path.exists());
    assert!(!credential_path.exists());
    assert!(!operation_path.exists());
    let mut input = FakeInput {
        selected: None,
        confirmed: true,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    let error = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: true,
            verbose: false,
        },
        &mut input,
    )
    .unwrap_err();

    assert!(error.to_string().contains("no stored model target matches"));
    assert!(input.selections.is_empty());
    assert!(input.summaries.is_empty());
    assert!(!connection_path.exists());
    assert!(!credential_path.exists());
    assert!(!operation_path.exists());
}

// 마지막 stored model을 대화형으로 제거하는 preview는 credential remove뿐 아니라
// 해당 complete binding에 귀속된 기존 Session이 native resume되지 않을 위험도 명시합니다.
#[test]
fn last_binding_preview_warns_about_remove_continuation_risk() {
    let fixture = Fixture::new("remove-resume-risk");
    let config_path = fixture.config_path("session: {}\n");
    fixture.seed_stored(&["alpha"], Some("alpha"));
    fixture.seed_credential();
    let mut input = FakeInput {
        selected: None,
        confirmed: false,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    let output = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: false,
            verbose: false,
        },
        &mut input,
    )
    .unwrap();

    assert_eq!(output, "Disconnect cancelled; nothing changed.\n");
    assert!(input.summaries[0].contains("Remove — no configured model uses vendor:team"));
    assert!(input.summaries[0].contains("Unavailable until this exact model is restored"));
    assert!(!input.summaries[0].contains("Connection being removed"));
    assert!(!input.summaries[0].contains("Still available for this account"));
}

// Catalog seed가 같은 account에 남는다면 explicit model의 마지막 행을 지워도 이후 catalog
// 선택과 discovery에 credential이 필요하므로 key를 함께 지우지 않습니다.
#[test]
fn last_explicit_binding_preserves_credential_for_a_stored_catalog_seed() {
    let fixture = Fixture::new("catalog-seed-preserves-credential");
    let repository = fixture.connections();
    let provider = ProviderId::new("qwencloud").unwrap();
    let account_id = AccountId::new("default").unwrap();
    let account = ConnectionAccount::new(
        provider.clone(),
        account_id.clone(),
        Some("QwenCloud".to_owned()),
        Some("Default".to_owned()),
    )
    .unwrap();
    let binding = StoredModelBinding::new(
        CompleteModelBinding::from_durable_json(
            r#"{"provider":"qwencloud","account":"default","model":"qwen3-coder-plus","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
        )
        .unwrap(),
        Some("Qwen3 Coder Plus".to_owned()),
    )
    .unwrap();
    let seed = ConnectionCatalogSeed::built_in(
        VersionedProfileId::new("qwencloud-token-plan-team-intl/v1").unwrap(),
        provider.clone(),
        account_id.clone(),
        Some("QwenCloud".to_owned()),
        Some("Default".to_owned()),
    )
    .unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_group_replace(account, vec![binding], Some(seed))
        .unwrap();
    repository.commit(&mutation).unwrap();
    let selection = ModelSelection::new(
        provider,
        account_id,
        ModelId::new("qwen3-coder-plus").unwrap(),
    );

    let plan = ExternalDisconnectPlan::prepare(
        &repository.capture().unwrap(),
        &selection,
        &StartupPolicy::initial(),
        false,
    )
    .unwrap();
    let preview = plan
        .preview
        .render(crate::interaction::connection::default_width())
        .unwrap();

    assert_eq!(
        plan.credential_action,
        ExternalDisconnectCredentialAction::Preserve
    );
    assert!(preview.contains("Keep — still used by the stored catalog definition"));
}

struct ConfigChangingInput {
    config_path: PathBuf,
    confirmation_reads: usize,
}

impl ExternalDisconnectInput for ConfigChangingInput {
    fn select_target(&mut self, _: &[String]) -> Result<String, AppError> {
        Err(AppError::message("the unique target must not prompt"))
    }

    fn confirm(
        &mut self,
        _: &dyn crate::interaction::connection::ConfirmationView,
    ) -> Result<bool, AppError> {
        self.confirmation_reads += 1;
        fs::write(&self.config_path, "tui:\n  max_fps: 60\n").unwrap();
        Ok(true)
    }
}

// 사람이 preview를 확인하는 동안 config.yaml이 바뀌면 exact command snapshot guard가
// journal/public/credential 쓰기보다 먼저 실패하고 세 저장소를 원래 상태로 유지합니다.
#[test]
fn changed_config_after_confirmation_aborts_before_disconnect_intent() {
    let fixture = Fixture::new("config-change");
    let config_path = fixture.config_path("session: {}\n");
    fixture.seed_stored(&["alpha"], Some("alpha"));
    fixture.seed_credential();
    let before_public = fs::read(fixture.connections().path()).unwrap();
    let before_credential = fs::read(fixture.credentials().path()).unwrap();
    let mut input = ConfigChangingInput {
        config_path: config_path.clone(),
        confirmation_reads: 0,
    };

    let error = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: Some("vendor".to_owned()),
            account: Some("team".to_owned()),
            yes: false,
            verbose: false,
        },
        &mut input,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("changed while this command was preparing")
    );
    assert_eq!(input.confirmation_reads, 1);
    assert_eq!(
        fs::read(fixture.connections().path()).unwrap(),
        before_public
    );
    assert_eq!(
        fs::read(fixture.credentials().path()).unwrap(),
        before_credential
    );
    assert!(!fixture.root.join("connection-operation.yaml").exists());
}

// success renderer가 width=1의 두 셀 grapheme 오류를 반환하면 execution은 commit 전에
// 해당 원인을 보존한 formatting error를 돌려주고 세 저장소를 그대로 둡니다.
#[test]
fn width_one_success_failure_precedes_disconnect_commit() {
    let fixture = Fixture::new("width-one-success");
    let config_path = fixture.config_path("session: {}\n");
    fixture.seed_stored(&["alpha", "한"], Some("한"));
    fixture.seed_credential();
    let before_public = fs::read(fixture.connections().path()).unwrap();
    let before_credential = fs::read(fixture.credentials().path()).unwrap();
    let mut input = FakeInput {
        selected: Some("vendor:team:alpha".to_owned()),
        confirmed: true,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    let error = execute_external_disconnect_with_success(
        &config_path,
        DisconnectCommand {
            provider: None,
            account: None,
            yes: false,
            verbose: false,
        },
        &mut input,
        |_, _, _| {
            Err(PresentationError::GraphemeExceedsWidth {
                grapheme_width: 2,
                width: 1,
            })
        },
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("formatting the disconnect success")
    );
    assert!(error.to_string().contains("2-cell"));
    assert_eq!(
        fs::read(fixture.connections().path()).unwrap(),
        before_public
    );
    assert_eq!(
        fs::read(fixture.credentials().path()).unwrap(),
        before_credential
    );
    assert!(!fixture.root.join("connection-operation.yaml").exists());
}

// target과 y/n을 한 canonical PTY write로 미리 보내도 preview 직전 TCIFLUSH가 두 번째
// line을 버립니다. 새 답 전에는 종료·mutation이 없고 새 y만 commit하며 새 n은 취소합니다.
#[test]
fn queued_confirmation_line_cannot_authorize_disconnect() {
    for (queued, fresh, commits) in [(b'y', b'y', true), (b'n', b'n', false)] {
        let fixture = Fixture::new(if commits { "queued-y" } else { "queued-n" });
        let config_path = fixture.config_path("session: {}\n");
        fixture.seed_stored(&["alpha", "beta"], None);
        fixture.seed_credential();
        let before_public = fs::read(fixture.connections().path()).unwrap();
        let before_credential = fs::read(fixture.credentials().path()).unwrap();
        let pty = openpty(None, None).unwrap();
        let mut master = fs::File::from(pty.master);
        fcntl(&master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
        let (result_tx, result_rx) = mpsc::channel();
        let worker_config = config_path.clone();
        let worker = thread::spawn(move || {
            let mut input = TtyPrompt::with_terminal(fs::File::from(pty.slave));
            let result = execute_external_disconnect_with(
                &worker_config,
                DisconnectCommand {
                    provider: None,
                    account: None,
                    yes: false,
                    verbose: false,
                },
                &mut input,
            )
            .map_err(|error| error.to_string());
            result_tx.send(result).unwrap();
        });

        read_until(
            &mut master,
            b"Target: ",
            Instant::now() + Duration::from_secs(2),
        );
        let queued_input = format!("vendor:team:beta\n{}\n", char::from(queued));
        write_until(
            &mut master,
            queued_input.as_bytes(),
            Instant::now() + Duration::from_secs(2),
        );
        read_until(
            &mut master,
            b"Apply this disconnect plan? [y/N] ",
            Instant::now() + Duration::from_secs(2),
        );

        assert!(matches!(
            result_rx.recv_timeout(Duration::from_millis(250)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert_eq!(
            fs::read(fixture.connections().path()).unwrap(),
            before_public
        );
        assert_eq!(
            fs::read(fixture.credentials().path()).unwrap(),
            before_credential
        );
        assert!(!fixture.root.join("connection-operation.yaml").exists());

        write_until(
            &mut master,
            &[fresh, b'\n'],
            Instant::now() + Duration::from_secs(2),
        );
        let result = result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("disconnect did not finish after the fresh answer")
            .expect("disconnect returned an unexpected error");
        worker.join().unwrap();

        if commits {
            assert!(result.starts_with("✓ Disconnected"));
            assert_eq!(fixture.connections().capture().unwrap().models().len(), 1);
        } else {
            assert_eq!(result, "Disconnect cancelled; nothing changed.\n");
            assert_eq!(
                fs::read(fixture.connections().path()).unwrap(),
                before_public
            );
        }
        assert_eq!(
            fs::read(fixture.credentials().path()).unwrap(),
            before_credential
        );
        assert!(!fixture.root.join("connection-operation.yaml").exists());
    }
}

// 이전 operation journal이 손상돼 있으면 새 config/selection/TTY보다 recovery가 먼저
// 실패하여, 사용자가 새 disconnect를 승인해 기존 복구 문제를 덮는 일이 없습니다.
#[test]
fn pending_recovery_failure_precedes_new_disconnect_input() {
    let fixture = Fixture::new("recovery-first");
    let config_path = fixture.config_path("not valid: [");
    fs::write(fixture.root.join("connection-operation.yaml"), "pending\n").unwrap();
    let mut input = FakeInput {
        selected: None,
        confirmed: true,
        selections: Vec::new(),
        summaries: Vec::new(),
    };

    let error = execute_external_disconnect_with(
        &config_path,
        DisconnectCommand {
            provider: None,
            account: None,
            yes: false,
            verbose: false,
        },
        &mut input,
    )
    .unwrap_err();

    assert!(error.to_string().contains("pending connection operation"));
    assert!(!error.to_string().contains("invalid configuration"));
    assert!(input.selections.is_empty());
    assert!(input.summaries.is_empty());
}

fn read_until(file: &mut fs::File, needle: &[u8], deadline: Instant) -> Vec<u8> {
    let mut output = Vec::new();
    while !output.windows(needle.len()).any(|window| window == needle) {
        let mut buffer = [0_u8; 1024];
        match file.read(&mut buffer) {
            Ok(0) => panic!("PTY closed before expected output appeared"),
            Ok(count) => output.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "PTY output deadline expired");
                thread::yield_now();
            },
            Err(error) => panic!("reading PTY output failed: {error}"),
        }
    }
    output
}

fn write_until(file: &mut fs::File, mut bytes: &[u8], deadline: Instant) {
    while !bytes.is_empty() {
        match file.write(bytes) {
            Ok(0) => panic!("PTY stopped accepting input"),
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "PTY input deadline expired");
                thread::yield_now();
            },
            Err(error) => panic!("writing PTY input failed: {error}"),
        }
    }
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = crate::state::connection::canonical_test_temp_dir().join(format!(
            "yo-cli-disconnect-{}-{name}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn config_path(&self, contents: &str) -> PathBuf {
        let path = self.root.join("config.yaml");
        fs::write(&path, contents).unwrap();
        path
    }

    fn connections(&self) -> LocalConnectionRepository {
        LocalConnectionRepository::new(self.root.join("connections.yaml"))
    }

    fn credentials(&self) -> LocalCredentialRepository {
        LocalCredentialRepository::new(self.root.join("credentials.yaml"))
    }

    fn seed_stored(&self, models: &[&str], preference: Option<&str>) {
        let repository = self.connections();
        for model in models {
            let account = ConnectionAccount::new(
                provider(),
                account(),
                Some("Vendor".to_owned()),
                Some("Team".to_owned()),
            )
            .unwrap();
            let binding =
                StoredModelBinding::new(complete(model), Some((*model).to_owned())).unwrap();
            let mutation = repository
                .capture()
                .unwrap()
                .prepare_model_upsert(account, binding)
                .unwrap()
                .unwrap();
            repository.commit(&mutation).unwrap();
        }
        if let Some(model) = preference {
            let target = StartupTarget::Model(ModelSelection::new(
                provider(),
                account(),
                ModelId::new(model).unwrap(),
            ));
            let mutation = repository
                .capture()
                .unwrap()
                .prepare_preference(Some(target))
                .unwrap();
            if let Some(mutation) = mutation {
                repository.commit(&mutation).unwrap();
            }
        }
    }

    fn seed_credential(&self) {
        let repository = self.credentials();
        let mutation = repository.prepare_set(&provider(), &account()).unwrap();
        repository
            .commit(
                &mutation,
                Some(&ApiCredential::new("fixture-secret").unwrap()),
            )
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn provider() -> ProviderId {
    ProviderId::new("vendor").unwrap()
}

fn account() -> AccountId {
    AccountId::new("team").unwrap()
}

fn complete(model: &str) -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(&format!(
        r#"{{"provider":"vendor","account":"team","model":"{model}","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1"}}"#
    ))
    .unwrap()
}
