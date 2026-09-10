use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use yo_core::{
    InputReference, SessionId, SkillReferenceProviderPoll, SkillReferenceSearchRequest, UserInput,
};

use super::{
    PreparedLocalSkills, StartupFrontend, fork_descriptor, prepare_local_skills,
    require_exact_fork_selection, require_exact_print_resume_binding,
    require_supported_fork_binding,
};
use crate::state::config;

// 저장된 native profile과 현재 catalog가 달라 replacement가 필요해도 print resume은
// Backend를 시작하지 않으며, 같은 상태의 TUI resume이나 새 print Session 의미는
// 기존 공용 경로에 남겨 둡니다.
#[test]
fn print_resume_rejects_binding_replacement_before_startup() {
    let error = require_exact_print_resume_binding(StartupFrontend::Print, true, true).unwrap_err();
    assert!(error.to_string().contains("without replacement"));
    assert!(require_exact_print_resume_binding(StartupFrontend::Terminal, true, true).is_ok());
    assert!(require_exact_print_resume_binding(StartupFrontend::Print, false, true).is_ok());
    assert!(require_exact_print_resume_binding(StartupFrontend::Print, true, false).is_ok());
}

// 현재 native fork가 검증할 수 없는 managed-state와 delegated host는 startup 자원을 만들기
// 전에 거부하고, source epoch와 무관하게 지원하는 local exact profile만 허용합니다.
#[test]
fn fork_rejects_unsupported_backend_proof_before_assembly() {
    use yo_core::{
        BackendBindingEvidence, BackendIdentity, ContinuationStrategy, ReplayExecutor,
        ReplayProfile,
    };
    let binding = |kind, strategy| {
        BackendBindingEvidence::new(
            kind,
            "1.0.0",
            BackendIdentity::new("binding/v1", "account"),
            BackendIdentity::new("model/v1", "model"),
            BackendIdentity::new("locator/v1", "source"),
            strategy,
        )
    };
    let exact = ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: ReplayProfile::SemanticOnly,
    };
    assert!(require_supported_fork_binding(&binding("yo-managed-model", exact)).is_ok());
    assert!(require_supported_fork_binding(&binding("codex-app-server", exact)).is_err());
    assert!(
        require_supported_fork_binding(&binding(
            "yo-managed-model",
            ContinuationStrategy::BackendManagedState,
        ))
        .is_err()
    );
    assert!(
        require_supported_fork_binding(&binding(
            "yo-managed-model",
            ContinuationStrategy::ExactReplay {
                executor: ReplayExecutor::LocalClient,
                replay_profile: ReplayProfile::ProviderPrivateLocalPlaintext,
            }
        ))
        .is_ok()
    );
}

// 저장된 tool/model 계약이 현재 catalog에서 replacement를 요구하면 child로 자동 변경하지
// 않습니다. 현재 계약과 같은 native 선택만 기존 startup 경로로 넘깁니다.
#[test]
fn fork_selection_rejects_replacement_and_host_fallback() {
    use crate::execution::{model::StartupBackend, tools::LocalToolRegistryRevision};
    let native = |replace_binding| StartupBackend::Native {
        provider: yo_core::ProviderId::new("provider").unwrap(),
        account: yo_core::AccountId::new("account").unwrap(),
        model: yo_core::ModelId::new("model").unwrap(),
        replace_binding,
        registry_revision: LocalToolRegistryRevision::NoTools,
        execution_manifest_digest: None,
    };
    assert!(require_exact_fork_selection(&native(false)).is_ok());
    assert!(require_exact_fork_selection(&native(true)).is_err());
    assert!(require_exact_fork_selection(&StartupBackend::Host(yo_core::HostId::codex())).is_err());
}

// child는 새 UUID를 받지만 parent의 workspace host/path를 보존하며 다른 host나 다른 디렉터리로
// 현재 fork를 암묵 이동하지 않습니다.
#[test]
fn fork_descriptor_preserves_workspace_and_allocates_an_independent_identity() {
    let workspace = SkillWorkspace::new();
    let other = SkillWorkspace::new();
    let host = "27049dbe-c29a-4615-90f7-1aaf288709ed".parse().unwrap();
    let other_host = "27049dbe-c29a-4615-90f7-1aaf288709ee".parse().unwrap();
    let parent = yo_core::SessionDescriptor::new(
        host,
        yo_core::HostWorkspacePath::normalize_local(&workspace.0).unwrap(),
    )
    .unwrap();
    let child = fork_descriptor(&workspace.0, host, &parent).unwrap();
    assert_ne!(child.session_id(), parent.session_id());
    assert_eq!(child.workspace_host_id(), parent.workspace_host_id());
    assert_eq!(child.workspace_path(), parent.workspace_path());
    assert!(fork_descriptor(&workspace.0, other_host, &parent).is_err());
    assert!(fork_descriptor(&other.0, host, &parent).is_err());
}

struct SkillWorkspace(PathBuf);

impl SkillWorkspace {
    fn new() -> Self {
        let identity = SessionId::new().unwrap();
        let path = std::env::temp_dir().join(format!("yo-startup-skills-{identity}"));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
}

impl Drop for SkillWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// 상대 스킬 경로는 프로세스 cwd 대신 전달된 저장 workspace에 연결되고 같은 본문을 준비합니다.
#[test]
fn local_skill_startup_uses_selected_workspace_and_preserves_prepared_instructions() {
    let workspace = SkillWorkspace::new();
    let skill_dir = workspace.0.join(".skills/review");
    fs::create_dir_all(&skill_dir).unwrap();
    let skill = skill_dir.join("SKILL.md");
    let body = "---\nname: review\ndescription: Check changes\n---\n한글 검토 지침\n";
    fs::write(&skill, body).unwrap();
    let config_path = workspace.0.join("config.yaml");
    fs::write(
        &config_path,
        "skills:\n  roots:\n    - path: .skills\n      scope: workspace\n",
    )
    .unwrap();
    let config = config::load_from(&config_path).unwrap();
    let host = "27049dbe-c29a-4615-90f7-1aaf288709ed".parse().unwrap();
    let PreparedLocalSkills {
        admission,
        references: provider,
    } = prepare_local_skills(&config, &workspace.0, host, StartupFrontend::Terminal).unwrap();
    let mut provider = provider.expect("terminal configuration exposes local discovery");
    provider
        .search(SkillReferenceSearchRequest::new(
            1,
            1,
            1,
            0..1,
            "$",
            "review",
            true,
        ))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let update = loop {
        match provider.poll().unwrap() {
            SkillReferenceProviderPoll::Update(update) => break update,
            SkillReferenceProviderPoll::Pending => {
                assert!(Instant::now() < deadline, "local skill discovery timed out");
                thread::sleep(Duration::from_millis(5));
            },
        }
    };
    let selected = update
        .candidates()
        .iter()
        .find(|entry| entry.reference().name() == "review")
        .expect("relative root was resolved in the selected workspace")
        .reference()
        .clone();
    let input =
        UserInput::with_references("$review", vec![InputReference::skill(0..7, selected)]).unwrap();
    let prepared = admission.prepare(&input).unwrap().unwrap();
    assert_eq!(prepared.instructions(), body);
    fs::remove_file(&skill).unwrap();
    assert!(admission.prepare(&input).is_err());
    assert_eq!(prepared.instructions(), body);
    drop(provider);

    fs::remove_dir(&skill_dir).unwrap();
    fs::remove_dir(workspace.0.join(".skills")).unwrap();
    let PreparedLocalSkills {
        admission,
        references: provider,
    } = prepare_local_skills(&config, &workspace.0, host, StartupFrontend::Print).unwrap();
    assert!(
        provider.is_none(),
        "print startup must not start UI discovery"
    );
    assert!(
        admission
            .prepare(&UserInput::new("ordinary text"))
            .unwrap()
            .is_none()
    );
}
