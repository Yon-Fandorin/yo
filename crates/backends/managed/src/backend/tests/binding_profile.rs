use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime},
};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    AgentCommand, AgentEvent, AgentIntent, AgentSession, AgentSessionPoll, ApiDialect,
    BackendCommandEvidence, BackendIdentity, CommandAdmission, EffectiveModelProfile,
    HostWorkspacePath, ModelConnectorEvent, ModelProfileLayer, ModelProfileParameters,
    ReasoningEffort, SessionDescriptor, ToolApprovalRequirement, ToolRegistry, TranscriptRecord,
    TurnOutcome, UserInput, VersionedProfileId, WorkspaceHostId,
    session_repository::{
        LocalSessionReader, LocalSessionRepository, read_stored_session_continuation,
    },
};

use super::support::{
    ExactAdmission, FixedTokenCounter, MockConnector, MockHost, binding, completed,
    context_profile, event_rounds, fixture_session, mock_tokenization_payload, registry, turn,
};
use crate::backend::{
    NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices,
    identity::semantically_equal_native_binding_identity,
};

fn parameters(value: &str) -> ModelProfileParameters {
    serde_json::from_str(value).unwrap()
}

fn profile(reasoning: &str, optional: &str, policy: &str) -> EffectiveModelProfile {
    profile_with_output(reasoning, optional, policy, Some(4_096))
}

fn profile_with_output(
    reasoning: &str,
    optional: &str,
    policy: &str,
    max_output_tokens: Option<u64>,
) -> EffectiveModelProfile {
    EffectiveModelProfile::resolve(
        None,
        &ModelProfileLayer::new(
            Some(ApiDialect::OpenAiResponses),
            Some(VersionedProfileId::new("test-tokenizer/v1").unwrap()),
            Some(1_000_000),
            max_output_tokens,
            Some(parameters(reasoning)),
            Some(parameters(optional)),
            Some(VersionedProfileId::new(policy).unwrap()),
        ),
    )
    .unwrap()
}

fn backend_with_profile(
    profile: EffectiveModelProfile,
) -> Result<NativeModelBackend, yo_core::BackendFailure> {
    backend_with_profile_and_registry(
        profile,
        registry(ToolApprovalRequirement::Automatic),
        Arc::new(Mutex::new(Vec::new())),
    )
}

fn backend_with_profile_and_registry(
    profile: EffectiveModelProfile,
    registry: yo_core::FrozenToolRegistry,
    requests: Arc<Mutex<Vec<yo_core::ModelConnectorRequest>>>,
) -> Result<NativeModelBackend, yo_core::BackendFailure> {
    backend_with_profile_registry_and_config(
        profile,
        registry,
        requests,
        NativeModelBackendConfig::default(),
    )
}

fn backend_with_profile_registry_and_config(
    profile: EffectiveModelProfile,
    registry: yo_core::FrozenToolRegistry,
    requests: Arc<Mutex<Vec<yo_core::ModelConnectorRequest>>>,
    config: NativeModelBackendConfig,
) -> Result<NativeModelBackend, yo_core::BackendFailure> {
    let model_context = profile.context().clone();
    NativeModelBackend::with_connector_and_profile(
        Box::new(MockConnector {
            rounds: event_rounds(vec![Vec::new()]),
            requests,
        }),
        binding(),
        registry,
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        model_context,
        Some(profile),
        config,
    )
}

fn backend_without_profile() -> NativeModelBackend {
    NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(Vec::new()),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap()
}

// 기존 generic 검증이 허용하는 profile도 필수 주입 admission의 거절을 보존하며,
// backend 생성 실패를 다른 validator로 우회하거나 connector 요청을 시작하지 않습니다.
#[test]
fn injected_binding_admission_rejection_precedes_connector_requests() {
    let profile = profile("{}", "{}", "local-tools/v1");
    let expected = yo_core::CompleteModelBinding::new(binding(), profile.clone()).unwrap();
    assert!(yo_core::admit_standard_complete_binding(&expected).is_ok());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let admission_calls = Arc::clone(&calls);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let admission = move |complete: &yo_core::CompleteModelBinding| {
        assert_eq!(complete, &expected);
        admission_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Err("injected binding rejection".to_owned())
    };
    let result = NativeModelBackend::with_connector_and_profile(
        Box::new(MockConnector {
            rounds: event_rounds(Vec::new()),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(admission),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        profile.context().clone(),
        Some(profile),
        NativeModelBackendConfig::default(),
    );
    let error = result
        .err()
        .expect("injected admission must reject initialization");
    assert_eq!(error.kind(), yo_core::BackendFailureKind::Initialization);
    assert_eq!(error.message(), "injected binding rejection");
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(requests.lock().unwrap().is_empty());
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("yo-managed-resume-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn resume_through_durable_agent_session(
    first_backend: NativeModelBackend,
    resumed_backend: NativeModelBackend,
) {
    let (directory, continuation) = durable_continuation(first_backend);
    let repository = LocalSessionRepository::open(&directory.0, 1024 * 1024).unwrap();
    let mut resumed = AgentSession::start_cancellable_with_continuation(
        resumed_backend,
        continuation,
        repository,
        || false,
    )
    .unwrap()
    .unwrap();
    resumed.shutdown().unwrap();
}

fn durable_continuation(
    mut first_backend: NativeModelBackend,
) -> (
    TestDirectory,
    yo_core::session_repository::StoredSessionContinuation,
) {
    first_backend.connector = Box::new(MockConnector {
        rounds: event_rounds(vec![vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "resume-fixture".to_owned(),
            },
            ModelConnectorEvent::TextDelta {
                output_index: 0,
                item_id: "message".to_owned(),
                content_index: 0,
                delta: "durable answer".to_owned(),
            },
            ModelConnectorEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
            completed("resume-fixture"),
        ]]),
        requests: Arc::new(Mutex::new(Vec::new())),
    });
    let directory = TestDirectory::new();
    let descriptor = SessionDescriptor::new(
        WorkspaceHostId::new().unwrap(),
        HostWorkspacePath::normalize_local(std::env::current_dir().unwrap()).unwrap(),
    )
    .unwrap();
    let session_id = descriptor.session_id();
    let repository = LocalSessionRepository::open(&directory.0, 1024 * 1024).unwrap();
    let mut session = AgentSession::start_cancellable_with_repository(
        first_backend,
        descriptor,
        repository,
        || false,
    )
    .unwrap()
    .unwrap();

    let mut admission = session
        .dispatch(AgentIntent::submit("durable request").unwrap())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while let CommandAdmission::Backpressured(pending) = admission {
        assert!(
            Instant::now() < deadline,
            "resume fixture stayed backpressured"
        );
        thread::sleep(Duration::from_millis(1));
        admission = session.retry(pending).unwrap();
    }

    let transcript = session.transcript_reader();
    loop {
        if transcript.read_after(None).entries().iter().any(|entry| {
            matches!(
                entry.record(),
                TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                    outcome: TurnOutcome::Completed,
                    ..
                })
            )
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "resume fixture Turn did not finish"
        );
        assert_ne!(session.poll().unwrap(), AgentSessionPoll::Closed);
        thread::sleep(Duration::from_millis(1));
    }
    session.shutdown().unwrap();
    drop(session);

    let reader = LocalSessionReader::open(&directory.0).unwrap();
    let continuation = read_stored_session_continuation(&reader, session_id).unwrap();
    drop(reader);
    (directory, continuation)
}

// explicit profile로 시작한 native backend는 profile의 reasoning effort를 실제 request
// 설정에 적용하고, durable identity에 endpoint와 여덟 resolved 필드를 모두 기록합니다.
#[test]
fn explicit_profile_controls_reasoning_and_complete_binding_identity() {
    let backend =
        backend_with_profile(profile(r#"{"effort":"high"}"#, "{}", "local-tools/v1")).unwrap();

    assert_eq!(backend.config.reasoning_effort, Some(ReasoningEffort::High));
    let evidence = backend.binding_evidence(fixture_session(7));
    assert_eq!(
        evidence.binding_identity().schema(),
        "yo.complete-model-binding/v1"
    );
    let value: serde_json::Value =
        serde_json::from_str(evidence.binding_identity().value()).unwrap();
    assert_eq!(value["api_dialect"], "openai-responses");
    assert_eq!(value["reasoning_parameters"]["effort"], "high");
    assert_eq!(value["optional_request_parameters"], serde_json::json!({}));
    assert_eq!(value["tool_capability_policy"], "local-tools/v1");
}

// output hard maximum이 unknown인 complete profile은 durable native identity에
// `max_output_tokens:null`을 만들지 않고 key 자체를 생략해 absence를 그대로 보존합니다.
#[test]
fn unknown_output_maximum_is_omitted_from_complete_binding_identity() {
    let backend = backend_with_profile(profile_with_output(
        r#"{"effort":"high"}"#,
        "{}",
        "local-tools/v1",
        None,
    ))
    .unwrap();
    let evidence = backend.binding_evidence(fixture_session(7));
    let value: serde_json::Value =
        serde_json::from_str(evidence.binding_identity().value()).unwrap();

    assert!(value.get("max_output_tokens").is_none());
    assert!(
        !evidence
            .binding_identity()
            .value()
            .contains("max_output_tokens")
    );
}

// runtime이 아직 보내지 못하는 optional parameter나 알 수 없는 policy/profile은 설정
// 단계에서 조용히 무시하지 않고 backend 초기화를 명시적으로 실패시킵니다.
#[test]
fn explicit_profile_rejects_unsupported_runtime_fields() {
    for unsupported in [
        profile("{}", r#"{"temperature":1.0}"#, "local-tools/v1"),
        profile(
            "{}",
            r#"{"thinking":{"type":"disabled"}}"#,
            "local-tools/v1",
        ),
        profile(r#"{"effort":"low"}"#, "{}", "local-tools/v1"),
        profile(r#"{"effort":"max"}"#, "{}", "local-tools/v1"),
        profile("{}", "{}", "other-tools/v1"),
        profile("null", "{}", "local-tools/v1"),
    ] {
        assert!(backend_with_profile(unsupported).is_err());
    }
}

// generic OpenAI binding은 Kimi private replay profile을 runtime 기본값으로 축약하지 않고
// complete-binding admission에서 초기화를 실패시켜 만족시킬 수 없는 private epoch를 열지 않습니다.
#[test]
fn generic_binding_rejects_cross_dialect_private_replay_profile() {
    let private = EffectiveModelProfile::resolve(
        None,
        &ModelProfileLayer::new(
            Some(ApiDialect::OpenAiResponses),
            Some(VersionedProfileId::new("test-tokenizer/v1").unwrap()),
            Some(1_000_000),
            Some(4_096),
            Some(parameters("{}")),
            Some(parameters("{}")),
            Some(VersionedProfileId::new("local-tools/v1").unwrap()),
        )
        .with_replay_profile(Some(
            VersionedProfileId::new("kimi-private-local-plaintext/v1").unwrap(),
        )),
    )
    .unwrap();

    assert!(backend_with_profile(private).is_err());
}

// no-tools profile은 empty runtime registry만 허용하고 실제 첫 model request에서도 현재
// tools와 tool_choice를 생략합니다. 같은 profile에 non-empty registry를 주면 fail-closed
// 하여 policy와 request-local exposure가 어긋나지 않습니다.
#[test]
fn no_tools_profile_requires_an_empty_registry_and_disables_request_exposure() {
    let no_tools = profile("{}", "{}", "no-tools/v1");
    assert!(backend_with_profile(no_tools.clone()).is_err());

    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = backend_with_profile_and_registry(
        no_tools,
        ToolRegistry::default().freeze(),
        Arc::clone(&requests),
    )
    .unwrap();
    assert!(backend.registry.is_empty());
    assert!(!backend.tool_exposure_enabled);
    assert!(backend.contract.tools().is_empty());

    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    assert!(matches!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("plain text request"),
            })
            .unwrap(),
        BackendCommandEvidence::RequestAccepted(_)
    ));
    let requests = requests.lock().unwrap();
    let body = mock_tokenization_payload(&requests[0], "qwen3.8max");
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
}

// local-tools/v1은 binding의 durable maximum으로 유지하면서 Session이 empty registry로
// 좁힐 수 있고, 그 조합은 실제 request-local exposure를 disabled로 투영합니다.
#[test]
fn local_tools_profile_accepts_an_empty_session_registry() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = backend_with_profile_and_registry(
        profile("{}", "{}", "local-tools/v1"),
        ToolRegistry::default().freeze(),
        Arc::clone(&requests),
    )
    .unwrap();
    assert!(backend.registry.is_empty());
    assert!(!backend.tool_exposure_enabled);
    assert!(backend.contract.tools().is_empty());

    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("restricted request"),
        })
        .unwrap();
    let requests = requests.lock().unwrap();
    let body = mock_tokenization_payload(&requests[0], "qwen3.8max");
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
}

// legacy catalog entry는 새 profile을 추정하지 않고 기존 yo.model-binding/v1 identity와
// caller가 준 reasoning 설정을 그대로 유지해 이전 Session resume 의미를 보존합니다.
#[test]
fn legacy_backend_keeps_the_existing_binding_identity() {
    let backend = backend_without_profile();

    assert_eq!(
        backend
            .binding_evidence(fixture_session(8))
            .binding_identity()
            .schema(),
        "yo.model-binding/v1"
    );
    assert_eq!(
        backend.config.reasoning_effort,
        Some(ReasoningEffort::Medium)
    );
}

// complete identity의 JSON key 순서가 달라도 typed 값이 같으면 native resume이 이를
// 다시 raw-byte 비교로 거절하지 않고 durable identity 그대로 runtime에 반환합니다.
#[test]
fn complete_resume_preserves_semantically_equal_durable_identity_bytes() {
    let mut first_backend =
        backend_with_profile(profile(r#"{"effort":"high"}"#, "{}", "local-tools/v1")).unwrap();
    let durable = BackendIdentity::new(
        "yo.complete-model-binding/v1",
        r#"{"tool_capability_policy":"local-tools/v1","optional_request_parameters":{},"reasoning_parameters":{"effort":"high"},"max_output_tokens":4096,"input_token_limit":1000000,"tokenizer_profile":"test-tokenizer/v1","api_dialect":"openai-responses","base_url":"https://example.invalid/v1","connector":"openai-responses","model":"qwen3.8max","account":"default","provider":"qwencloud"}"#,
    );
    first_backend.binding_identity = durable;
    let resumed_backend =
        backend_with_profile(profile(r#"{"effort":"high"}"#, "{}", "local-tools/v1")).unwrap();

    resume_through_durable_agent_session(first_backend, resumed_backend);
}

// native resume의 core 비교기도 CLI 전처리에 기대지 않고 범위 밖 integer와 유한하지
// 않은 float spelling을 거절해, 두 malformed identity를 같은 값으로 인정하지 않습니다.
#[test]
fn complete_resume_identity_rejects_closed_number_admission_failures() {
    let backend =
        backend_with_profile(profile(r#"{"effort":"high"}"#, "{}", "local-tools/v1")).unwrap();
    let evidence = backend.binding_evidence(fixture_session(7));
    let canonical = evidence.binding_identity().value();

    for invalid in ["18446744073709551616", "1e400"] {
        let value = canonical.replace(
            r#""reasoning_parameters":{"effort":"high"}"#,
            &format!(r#""reasoning_parameters":{{"value":{invalid}}}"#),
        );
        assert_ne!(value, canonical);
        let identity = BackendIdentity::new("yo.complete-model-binding/v1", value);
        assert!(!semantically_equal_native_binding_identity(
            &identity, &identity
        ));
    }
}

// legacy v1이 가진 알 수 없는 역사적 필드는 typed 좌표 비교에서 무시하고, 성공한
// resume은 runtime exact check를 위해 원래 durable evidence를 손실 없이 돌려줍니다.
#[test]
fn legacy_resume_preserves_valid_unknown_durable_fields() {
    let mut first_backend = backend_without_profile();
    let durable = BackendIdentity::new(
        "yo.model-binding/v1",
        r#"{"provider":"qwencloud","account":"default","model":"qwen3.8max","connector":"openai-responses","api_dialect":"openai-responses","base_url":"https://example.invalid/v1","historical":"retained"}"#,
    );
    first_backend.binding_identity = durable;

    resume_through_durable_agent_session(first_backend, backend_without_profile());
}

const COMMAND_SCHEMA: &str = "yo.managed-command-binding/v1";

fn manifest_digest(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}

fn command_backend(digest: Option<String>) -> NativeModelBackend {
    command_backend_for_model(digest, "qwen3.8max")
}

fn command_backend_for_model(digest: Option<String>, model: &str) -> NativeModelBackend {
    let saved = binding();
    let selected = yo_core::EffectiveModelBinding::new(
        saved.provider_id().clone(),
        saved.account_id().clone(),
        yo_core::ModelId::new(model).unwrap(),
        saved.api_dialect(),
        saved.endpoint().clone(),
    );
    let profile = profile("{}", "{}", "local-tools/v1");
    NativeModelBackend::with_connector_and_profile(
        Box::new(MockConnector {
            rounds: event_rounds(Vec::new()),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        selected,
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        profile.context().clone(),
        Some(profile),
        NativeModelBackendConfig {
            execution_manifest_digest: digest,
            ..NativeModelBackendConfig::default()
        },
    )
    .unwrap()
}

fn wrapped_identity(base: &BackendIdentity, digest: &str) -> BackendIdentity {
    BackendIdentity::new(
        COMMAND_SCHEMA,
        format!(
            r#"{{"model_binding":{{"schema":"{}","value":{}}},"execution_manifest_digest":"{digest}"}}"#,
            base.schema(),
            base.value(),
        ),
    )
}

// 명령 digest가 없으면 기존 bytes를 유지하고 no-tools는 제공된 digest도 노출하지 않는다.
#[test]
fn command_binding_keeps_legacy_and_no_tools_identity_bytes() {
    let legacy = backend_without_profile();
    assert_eq!(
        legacy.binding_identity.value(),
        r#"{"account":"default","api_dialect":"openai-responses","base_url":"https://example.invalid/v1","connector":"openai-responses","model":"qwen3.8max","provider":"qwencloud"}"#
    );
    let (decoded, digest) =
        NativeModelBackend::decode_binding_identity(&legacy.binding_identity).unwrap();
    assert_eq!(decoded, legacy.binding_identity);
    assert_eq!(digest, None);

    let make_no_tools = |digest| {
        backend_with_profile_registry_and_config(
            profile("{}", "{}", "no-tools/v1"),
            ToolRegistry::default().freeze(),
            Arc::new(Mutex::new(Vec::new())),
            NativeModelBackendConfig {
                execution_manifest_digest: digest,
                ..NativeModelBackendConfig::default()
            },
        )
        .unwrap()
    };
    assert_eq!(
        make_no_tools(None).binding_identity,
        make_no_tools(Some(manifest_digest('a'))).binding_identity
    );
    let base = command_backend(None).binding_identity;
    let wrapped = command_backend(Some(manifest_digest('a'))).binding_identity;
    assert_eq!(
        NativeModelBackend::decode_binding_identity(&wrapped).unwrap(),
        (base, Some(manifest_digest('a')))
    );
}

// wrapper와 두 envelope는 object만 허용하고 중복·unknown·null·중첩 wrapper를 거절한다.
#[test]
fn command_binding_rejects_non_objects_duplicates_unknowns_and_nulls() {
    let legacy = backend_without_profile().binding_identity;
    let valid = wrapped_identity(&legacy, &manifest_digest('a'));
    let base = legacy.value();
    let model = format!(r#"{{"schema":"{}","value":{base}}}"#, legacy.schema());
    let digest = manifest_digest('a');
    let invalid = vec![
        format!(r#"[{model},"{digest}"]"#),
        format!(r#"{{"model_binding":["{}",{base}],"execution_manifest_digest":"{digest}"}}"#, legacy.schema()),
        valid.value().replace(base, r#"["qwencloud","default","qwen3.8max","openai-responses","openai-responses","https://example.invalid/v1"]"#),
        valid.value().replace(&model, "null"),
        valid.value().replace(base, "null"),
        valid.value().replace(base, &serde_json::to_string(base).unwrap()),
        valid.value().replace(&format!(r#""{digest}""#), "null"),
        valid.value().replace(r#""schema":"yo.model-binding/v1""#, r#""schema":null"#),
        format!(r#"{{"model_binding":{model},"model_binding":{model},"execution_manifest_digest":"{digest}"}}"#),
        format!(r#"{{"model_binding":{model},"execution_manifest_digest":"{digest}","execution_manifest_digest":"{digest}"}}"#),
        valid.value().replacen('{', r#"{"extra":null,"#, 1),
        valid.value().replace(r#""schema":"yo.model-binding/v1""#, r#""schema":"yo.model-binding/v1","extra":null"#),
        valid.value().replace(r#""schema":"yo.model-binding/v1""#, r#""schema":"yo.model-binding/v1","schema":"yo.model-binding/v1""#),
        valid.value().replace(r#""value":"#, &format!(r#""value":{base},"value":"#)),
        valid.value().replace(r#""provider":"qwencloud""#, r#""provider":"qwencloud","provider":"qwencloud""#),
        valid.value().replace(r#""provider":"qwencloud""#, r#""provider":"qwencloud","extra":1"#),
        wrapped_identity(&valid, &digest).value().to_owned(),
    ];
    for value in invalid {
        assert!(
            NativeModelBackend::decode_binding_identity(&BackendIdentity::new(
                COMMAND_SCHEMA,
                value.clone()
            ))
            .is_err(),
            "accepted {value}"
        );
    }
    for digest in [
        String::new(),
        "sha256:".to_owned(),
        manifest_digest('A'),
        manifest_digest('g'),
        format!("{}0", manifest_digest('a')),
    ] {
        assert!(
            NativeModelBackend::decode_binding_identity(&wrapped_identity(&legacy, &digest))
                .is_err()
        );
        assert!(
            super::super::identity::native_binding_identity(&binding(), None, Some(&digest))
                .is_err()
        );
    }
}

// complete decoder의 숫자 spelling과 recursive parameter 규칙을 wrapper도 그대로 적용한다.
#[test]
fn command_binding_preserves_complete_number_grammar_and_recursive_duplicates() {
    let base = command_backend(None).binding_identity;
    let value = base.value();
    for invalid_parameters in [
        r#"{"x":18446744073709551616}"#,
        r#"{"x":-9223372036854775809}"#,
        r#"{"x":1e400}"#,
        r#"{"x":[{"same":1,"same":2}]}"#,
    ] {
        let invalid = BackendIdentity::new(
            base.schema(),
            value.replace(
                r#""reasoning_parameters":{}"#,
                &format!(r#""reasoning_parameters":{invalid_parameters}"#),
            ),
        );
        assert!(
            NativeModelBackend::decode_binding_identity(&wrapped_identity(
                &invalid,
                &manifest_digest('a')
            ))
            .is_err()
        );
    }
    for (from, to) in [
        (r#""max_output_tokens":4096"#, r#""max_output_tokens":null"#),
        (
            r#""model":"qwen3.8max""#,
            r#""model":"qwen3.8max","unknown":null"#,
        ),
        (
            r#""model":"qwen3.8max""#,
            r#""model":"qwen3.8max","model":"qwen3.8max""#,
        ),
    ] {
        let invalid = BackendIdentity::new(base.schema(), value.replace(from, to));
        assert!(
            NativeModelBackend::decode_binding_identity(&wrapped_identity(
                &invalid,
                &manifest_digest('a')
            ))
            .is_err()
        );
    }
    let number_binding = |number| {
        wrapped_identity(
            &BackendIdentity::new(
                base.schema(),
                value.replace(
                    r#""reasoning_parameters":{}"#,
                    &format!(r#""reasoning_parameters":{{"x":{number}}}"#),
                ),
            ),
            &manifest_digest('a'),
        )
    };
    let integer = number_binding("1");
    let float = number_binding("1.0");
    assert!(NativeModelBackend::decode_binding_identity(&integer).is_ok());
    assert!(NativeModelBackend::decode_binding_identity(&float).is_ok());
    assert!(!semantically_equal_native_binding_identity(
        &integer, &float
    ));
    assert!(semantically_equal_native_binding_identity(
        &number_binding("-0.0"),
        &number_binding("0.0")
    ));
}

// outer encoded UTF-8의 4096 byte는 허용하고 첫 초과는 JSON parsing 전에 거절한다.
#[test]
fn command_binding_enforces_the_complete_encoded_identity_limit() {
    let base = backend_without_profile().binding_identity;
    let valid = wrapped_identity(&base, &manifest_digest('a'));
    let padding = 4096 - valid.value().len();
    let at_limit = format!("{}{}", valid.value(), " ".repeat(padding));
    assert!(
        NativeModelBackend::decode_binding_identity(&BackendIdentity::new(
            COMMAND_SCHEMA,
            &at_limit
        ))
        .is_ok()
    );
    assert!(
        NativeModelBackend::decode_binding_identity(&BackendIdentity::new(
            COMMAND_SCHEMA,
            format!("{at_limit} ")
        ))
        .is_err()
    );

    let encode = |padding: usize| {
        let profile = profile(
            &format!(r#"{{"padding":"{}"}}"#, "x".repeat(padding)),
            "{}",
            "local-tools/v1",
        );
        super::super::identity::native_binding_identity(
            &binding(),
            Some(&profile),
            Some(&manifest_digest('a')),
        )
    };
    let padding = 4096 - encode(0).unwrap().value().len();
    assert_eq!(encode(padding).unwrap().value().len(), 4096);
    assert!(encode(padding + 1).is_err());

    let unicode = BackendIdentity::new(base.schema(), base.value().replace("qwen3.8max", "모델"));
    let wrapped = wrapped_identity(&unicode, &manifest_digest('a'));
    let at_limit = format!(
        "{}{}",
        wrapped.value(),
        " ".repeat(4096 - wrapped.value().len())
    );
    assert!(
        NativeModelBackend::decode_binding_identity(&BackendIdentity::new(
            COMMAND_SCHEMA,
            &at_limit
        ))
        .is_ok()
    );
    assert!(
        NativeModelBackend::decode_binding_identity(&BackendIdentity::new(
            COMMAND_SCHEMA,
            format!("{at_limit} ")
        ))
        .is_err()
    );
}

// 동일 digest는 실제 durable resume/fork를 허용하고 변경·누락 digest는 둘 다 거절한다.
#[test]
fn command_manifest_is_required_for_exact_resume_and_fork() {
    let (_directory, continuation) =
        durable_continuation(command_backend(Some(manifest_digest('a'))));
    for digest in [Some(manifest_digest('a')), Some(manifest_digest('b')), None] {
        let matches = digest.as_deref() == Some(manifest_digest('a').as_str());
        let mut resumed = command_backend(digest.clone());
        assert_eq!(
            resumed.resume_session(continuation.target()).is_ok(),
            matches
        );
        let candidate = command_backend(digest);
        let child = SessionDescriptor::new(
            WorkspaceHostId::new().unwrap(),
            HostWorkspacePath::normalize_local(std::env::current_dir().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            candidate.prepare_exact_fork(&continuation, child).is_ok(),
            matches
        );
    }
    resume_through_durable_agent_session(
        command_backend(Some(manifest_digest('a'))),
        command_backend(Some(manifest_digest('a'))),
    );
}

// 순서와 공백이 다른 durable wrapper도 semantic 검증 후 resume/fork하며 자식은 원본 bytes를
// 보존한다.
#[test]
fn command_fork_preserves_semantically_equal_durable_wrapper_bytes() {
    use yo_core::session_repository::StoredSessionReader;

    let digest = manifest_digest('a');
    let mut first = command_backend(Some(digest.clone()));
    let canonical = first.binding_identity.clone();
    let (base, _) = NativeModelBackend::decode_binding_identity(&canonical).unwrap();
    let base_value: serde_json::Value = serde_json::from_str(base.value()).unwrap();
    let durable = BackendIdentity::new(
        COMMAND_SCHEMA,
        format!(
            " {{\n  \"model_binding\" : {{ \"value\" : {}, \"schema\" : \"{}\" }},\n  \"execution_manifest_digest\" : \"{digest}\"\n}} ",
            serde_json::to_string_pretty(&base_value).unwrap(),
            base.schema(),
        ),
    );
    assert_ne!(canonical, durable);
    assert!(semantically_equal_native_binding_identity(
        &canonical, &durable
    ));
    first.binding_identity = durable.clone();
    let (directory, parent) = durable_continuation(first);
    assert_eq!(parent.target().binding().binding_identity(), &durable);
    let reader = LocalSessionReader::open(&directory.0).unwrap();
    let before = reader
        .read_session(parent.descriptor().session_id())
        .unwrap();

    let mut resumed = command_backend(Some(digest.clone()));
    let evidence = resumed.resume_session(parent.target()).unwrap();
    assert_eq!(evidence.binding_identity(), &durable);
    resumed.shutdown().unwrap();

    let child_descriptor = || {
        SessionDescriptor::new(
            WorkspaceHostId::new().unwrap(),
            HostWorkspacePath::normalize_local(std::env::current_dir().unwrap()).unwrap(),
        )
        .unwrap()
    };
    for mut incompatible in [
        command_backend(Some(manifest_digest('b'))),
        command_backend(None),
        command_backend_for_model(Some(digest.clone()), "different-model"),
    ] {
        assert!(
            incompatible
                .prepare_exact_fork(&parent, child_descriptor())
                .is_err()
        );
        assert!(incompatible.resume_session(parent.target()).is_err());
        assert!(incompatible.session.is_none());
    }

    let candidate = command_backend(Some(digest));
    let child = candidate
        .prepare_exact_fork(&parent, child_descriptor())
        .unwrap();
    let child_id = child.descriptor().session_id();
    assert_eq!(child.target().binding().binding_identity(), &durable);
    assert_eq!(
        child.target().model_replay(),
        parent.target().model_replay()
    );
    assert_ne!(
        child.target().binding().session_locator(),
        parent.target().binding().session_locator()
    );
    let repository = LocalSessionRepository::open(&directory.0, 1024 * 1024).unwrap();
    let mut live_child =
        AgentSession::start_cancellable_with_continuation(candidate, child, repository, || false)
            .unwrap()
            .unwrap();
    live_child.shutdown().unwrap();
    drop(live_child);
    let recovered_child = read_stored_session_continuation(&reader, child_id).unwrap();
    assert_eq!(
        recovered_child.target().binding().binding_identity(),
        &durable
    );
    assert_eq!(
        recovered_child.target().model_replay(),
        parent.target().model_replay()
    );
    assert_eq!(
        reader
            .read_session(parent.descriptor().session_id())
            .unwrap(),
        before
    );
}

// semantic model 교체는 같은 manifest를 허용하지만 변경·누락·새 manifest로 도구를 바꾸지 못한다.
#[test]
fn command_manifest_replacement_allows_a_new_model_only_with_the_same_digest() {
    for source_digest in [Some(manifest_digest('a')), None] {
        let (_directory, continuation) =
            durable_continuation(command_backend(source_digest.clone()));
        for target_digest in [Some(manifest_digest('a')), Some(manifest_digest('b')), None] {
            let mut replacement =
                command_backend_for_model(target_digest.clone(), "replacement-model");
            assert!(!semantically_equal_native_binding_identity(
                &replacement.binding_identity,
                continuation.target().binding().binding_identity()
            ));
            let result = replacement.resume_session_replacing_binding(continuation.target());
            assert_eq!(result.is_ok(), source_digest == target_digest);
            if let Ok(evidence) = result {
                assert_eq!(evidence.binding_identity(), &replacement.binding_identity);
            } else {
                assert!(replacement.session.is_none());
            }
        }
    }
}
