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
            CommandId::Exit,
        ]
    );
}
