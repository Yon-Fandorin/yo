use std::sync::{Arc, Mutex};

use super::{
    super::{
        AgentBackend, AgentCommand, BackendCommandEvidence, NativeModelBackend,
        NativeModelBackendConfig, NativeModelBackendServices,
        semantically_equal_native_binding_identity,
    },
    support::{
        ExactAdmission, FixedTokenCounter, MockConnector, MockHost, binding, context_profile,
        event_rounds, mock_tokenization_payload, registry, turn,
    },
};
use crate::{
    AgentRuntime, ApiDialect, BackendBindingEvidence, BackendIdentity, BackendResumeTarget,
    EffectiveModelProfile, JournalSequence, ModelProfileLayer, ModelProfileParameters, ModelReplay,
    ModelReplayDelta, ModelReplayItem, ModelReplayRole, ReasoningEffort, ToolApprovalRequirement,
    ToolRegistry, UserInput, VersionedProfileId, fixture_descriptor, fixture_session,
    journal::SessionJournal,
    session_repository::{
        AppendError, AppendReceipt, DurableRecord, RepositoryEntry, RepositoryError,
        RepositorySequence, SessionRepository,
    },
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
) -> Result<NativeModelBackend, crate::BackendFailure> {
    backend_with_profile_and_registry(
        profile,
        registry(ToolApprovalRequirement::Automatic),
        Arc::new(Mutex::new(Vec::new())),
    )
}

fn backend_with_profile_and_registry(
    profile: EffectiveModelProfile,
    registry: crate::FrozenToolRegistry,
    requests: Arc<Mutex<Vec<crate::ModelConnectorRequest>>>,
) -> Result<NativeModelBackend, crate::BackendFailure> {
    let model_context = profile.context().clone();
    NativeModelBackend::with_connector_and_profile(
        Box::new(MockConnector {
            rounds: event_rounds(vec![Vec::new()]),
            requests,
        }),
        binding(),
        registry,
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        model_context,
        Some(profile),
        NativeModelBackendConfig::default(),
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
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap()
}

fn resume_target(
    backend: &NativeModelBackend,
    binding_identity: BackendIdentity,
) -> BackendResumeTarget {
    let session_id = fixture_session(9);
    let current = backend.binding_evidence(session_id);
    let binding = BackendBindingEvidence::new(
        current.backend_kind(),
        current.backend_version(),
        binding_identity,
        current.model_identity().clone(),
        current.session_locator().clone(),
        current.continuation_strategy(),
    );
    let mut replay = ModelReplay::default();
    replay
        .apply(&ModelReplayDelta::new(
            Some(backend.contract.clone()),
            vec![ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: "durable request".to_owned(),
                refusal: None,
            }],
        ))
        .unwrap();
    BackendResumeTarget::new(session_id, 1, binding, JournalSequence::new(1))
        .with_model_replay(replay)
}

#[derive(Default)]
struct DurableTestRepository {
    next_sequence: u64,
}

impl SessionRepository for DurableTestRepository {
    fn append(
        &mut self,
        _session_id: crate::SessionId,
        _record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        self.next_sequence += 1;
        Ok(AppendReceipt::new(RepositorySequence::new(
            self.next_sequence,
        )))
    }

    fn read_after(
        &self,
        _session_id: crate::SessionId,
        _sequence: Option<RepositorySequence>,
        _limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        Ok(Vec::new())
    }
}

fn resume_runtime(backend: NativeModelBackend) -> AgentRuntime<NativeModelBackend> {
    let session_id = fixture_session(9);
    let journal = SessionJournal::with_repository_and_descriptor(
        Box::new(DurableTestRepository::default()),
        fixture_descriptor(session_id),
    );
    AgentRuntime::with_journal(backend, journal)
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
    let backend =
        backend_with_profile(profile(r#"{"effort":"high"}"#, "{}", "local-tools/v1")).unwrap();
    let durable = BackendIdentity::new(
        "yo.complete-model-binding/v1",
        r#"{"tool_capability_policy":"local-tools/v1","optional_request_parameters":{},"reasoning_parameters":{"effort":"high"},"max_output_tokens":4096,"input_token_limit":1000000,"tokenizer_profile":"test-tokenizer/v1","api_dialect":"openai-responses","base_url":"https://example.invalid/v1","connector":"openai-responses","model":"qwen3.8max","account":"default","provider":"qwencloud"}"#,
    );
    let target = resume_target(&backend, durable);
    let mut runtime = resume_runtime(backend);

    runtime.initialize_resume(&target).unwrap();
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
    let backend = backend_without_profile();
    let durable = BackendIdentity::new(
        "yo.model-binding/v1",
        r#"{"provider":"qwencloud","account":"default","model":"qwen3.8max","connector":"openai-responses","api_dialect":"openai-responses","base_url":"https://example.invalid/v1","historical":"retained"}"#,
    );
    let target = resume_target(&backend, durable);
    let mut runtime = resume_runtime(backend);

    runtime.initialize_resume(&target).unwrap();
}
