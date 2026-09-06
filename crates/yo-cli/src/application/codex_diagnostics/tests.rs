use super::{CodexWarningCollector, MAX_CODEX_COMPATIBILITY_WARNINGS};
use crate::diagnostic::CliDiagnostic;

// 같은 warning은 한 번만 남기고, 서로 다른 warning은 관측된 순서로 publication합니다.
#[test]
fn codex_warning_collector_deduplicates_and_preserves_observation_order() {
    let collector = CodexWarningCollector::default();
    collector.observe_message("first".to_owned());
    collector.observe_message("first".to_owned());
    collector.observe_message("second".to_owned());

    let diagnostics = collector.take_pending_diagnostics();
    assert_eq!(
        diagnostics
            .iter()
            .map(CliDiagnostic::message)
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    assert!(collector.take_pending_diagnostics().is_empty());
}

// 상한을 넘는 warning은 메모리와 stderr 모두 bounded하게 한 개의 suppression 진단으로
// 접습니다.
#[test]
fn codex_warning_collector_suppresses_distinct_overflow_once() {
    let collector = CodexWarningCollector::default();
    for index in 0..=MAX_CODEX_COMPATIBILITY_WARNINGS {
        collector.observe_message(format!("warning {index}"));
    }

    let diagnostics = collector.take_pending_diagnostics();
    assert_eq!(diagnostics.len(), MAX_CODEX_COMPATIBILITY_WARNINGS + 1);
    assert_eq!(
        diagnostics.last().map(CliDiagnostic::message),
        Some("additional Codex compatibility warnings were suppressed after 32 distinct warnings")
    );
    assert!(collector.take_pending_diagnostics().is_empty());
}

// stdout publication이 실패하면 이후에 도착한 Codex warning도 publication하지 않습니다.
#[test]
fn codex_warning_collector_discard_blocks_late_observations() {
    let collector = CodexWarningCollector::default();
    collector.observe_message("already pending".to_owned());
    collector.discard_pending();
    collector.observe_message("arrived after stdout failure".to_owned());

    assert!(collector.take_pending_diagnostics().is_empty());
}
