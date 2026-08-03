use super::{PanelSnapshot, SelectionEntry};
use crate::{
    overlay::selection::{PanelValidationError, TextField},
    surface::GraphemeError,
};

// title·entry가 비어 있으면 화면 state로 publication되기 전에 구조 오류로 거절한다.
#[test]
fn rejects_empty_required_structure() {
    assert_eq!(
        PanelSnapshot::new("", vec![SelectionEntry::enabled("a", "A", None)]),
        Err(PanelValidationError::EmptyTitle)
    );
    assert_eq!(
        PanelSnapshot::new("Title", vec![]),
        Err(PanelValidationError::EmptyEntries)
    );
}

// 한 snapshot 안의 opaque identity는 유일해야 late refresh와 accept가 같은 항목을
// 모호하게 가리키지 않는다.
#[test]
fn rejects_duplicate_entry_identities() {
    assert_eq!(
        PanelSnapshot::new(
            "Title",
            vec![
                SelectionEntry::enabled("same", "A", None),
                SelectionEntry::enabled("same", "B", None),
            ],
        ),
        Err(PanelValidationError::DuplicateIdentity { index: 1 })
    );
}

// 화면에 그릴 provider label의 control 문자는 한 행 geometry를 깨뜨리므로 publication
// 단계에서 field 위치와 원인을 보존해 거절한다.
#[test]
fn rejects_control_text_with_typed_field_context() {
    assert_eq!(
        PanelSnapshot::new("Title", vec![SelectionEntry::enabled("a", "A\nB", None)],),
        Err(PanelValidationError::UnsafeText {
            field: TextField::Label { index: 0 },
            cause: GraphemeError::Control,
        })
    );
}

// 화면에 표시하지 않는 opaque identity는 terminal grapheme 규칙을 적용하지 않고,
// 비어 있지 않고 snapshot 안에서 유일한지만 확인한다.
#[test]
fn opaque_identity_does_not_require_display_safe_text() {
    assert!(
        PanelSnapshot::new(
            "Title",
            vec![SelectionEntry::enabled("backend\nopaque", "Visible", None)],
        )
        .is_ok()
    );
}
