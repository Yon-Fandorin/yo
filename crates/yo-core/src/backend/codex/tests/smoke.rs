use crate::AgentBackend;

// 로컬에 호환되는 Codex가 설치된 환경에서는 실제 stdio app-server와 initialize를
// 완료하고 명시적 shutdown으로 자식 프로세스를 회수할 수 있는지 수동 검증한다.
#[test]
#[ignore = "requires a compatible local Codex installation and writable Codex state"]
fn local_codex_initializes_and_shuts_down() {
    let cwd = std::env::current_dir().unwrap();
    let mut backend =
        super::super::CodexBackend::spawn(super::super::CodexBackendConfig::new(cwd)).unwrap();

    backend.shutdown().unwrap();
}
