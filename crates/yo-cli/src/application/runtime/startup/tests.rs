use super::{StartupFrontend, require_exact_print_resume_binding};

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
