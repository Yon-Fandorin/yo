use super::CommandRegistry;
use crate::command::CommandId;

// registry filtering 결과는 각 module definition을 help, model, compact, exit 순으로 합성한
// 안정된 제품 순서를 그대로 보존한다.
#[test]
fn command_filter_preserves_module_declared_order() {
    assert_eq!(
        CommandRegistry::built_in()
            .matching("")
            .map(|definition| definition.id())
            .collect::<Vec<_>>(),
        vec![
            CommandId::Help,
            CommandId::Model,
            CommandId::Compact,
            CommandId::Changes,
            CommandId::Output,
            CommandId::Preview,
            CommandId::Attach,
            CommandId::Exit,
            CommandId::New,
            CommandId::Fork,
            CommandId::Tree,
            CommandId::Resume,
            CommandId::Prompt,
        ]
    );
}

// 도움말 명령 목록은 별도 사본 대신 실제 registry에서 생성하고 새 탐색·편집 조작도 제공한다.
#[test]
fn help_document_uses_registry_and_explains_interaction() {
    let registry = CommandRegistry::built_in();
    let document = registry.help_document();
    assert!(document.to_snapshot().is_some());
    for definition in registry.matching("") {
        assert!(
            document
                .markdown
                .contains(&format!("`{}`", definition.invocation()))
        );
    }
    for shortcut in [
        "Alt+Up/Down",
        "Alt+O",
        "Ctrl+O",
        "Ctrl+A/E",
        "Ctrl+U/K",
        "Ctrl+Y",
        "Shift+Tab",
    ] {
        assert!(document.markdown.contains(shortcut));
    }
}
