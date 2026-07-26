use super::super::{Attributes, Color, Style};

// attribute bit set이 여러 resolved 속성을 중복 없이 조합하고 조회하는지 확인한다.
#[test]
fn attributes_compose_as_a_compact_resolved_set() {
    let attributes = Attributes::BOLD
        .union(Attributes::ITALIC)
        .union(Attributes::BOLD);

    assert!(attributes.contains(Attributes::BOLD));
    assert!(attributes.contains(Attributes::ITALIC));
    assert!(!attributes.contains(Attributes::UNDERLINE));
    assert_eq!(
        attributes.bits(),
        Attributes::BOLD.bits() | Attributes::ITALIC.bits()
    );
}

// 기본 Style은 terminal-default 전경·배경과 비어 있는 attribute를 표현한다.
#[test]
fn default_style_is_fully_resolved_terminal_default() {
    assert_eq!(
        Style::default(),
        Style::new(Color::Default, Color::Default, Attributes::empty())
    );
}
