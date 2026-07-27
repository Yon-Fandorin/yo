use super::editor_with;
use crate::{
    input::editor::{PromptEditor, layout::LayoutError},
    prompt::{PromptMeasure, PromptMeasureError, measure},
};

// 빈 입력도 커서가 머물 한 행을 희망 높이로 보고한다.
#[test]
fn empty_prompt_desires_one_cursor_row() {
    let measurement = measure(&PromptEditor::new(), 80).unwrap();

    assert_eq!(
        measurement,
        PromptMeasure {
            desired_height: std::num::NonZeroU16::new(1).unwrap(),
        }
    );
}

// 측정은 현재 폭에서 줄바꿈된 내용과 끝 커서가 실제로 차지할 전체 높이를 보고한다.
#[test]
fn measurement_depends_on_wrapped_content_and_visible_cursor() {
    let editor = editor_with("abcd");

    let measurement = measure(&editor, 2).unwrap();

    assert_eq!(
        measurement.desired_height,
        std::num::NonZeroU16::new(3).unwrap()
    );
}

// 같은 편집 상태도 넓은 폭에서는 필요한 행 수가 줄어든다.
#[test]
fn measurement_reflows_when_width_changes() {
    let editor = editor_with("abcd");

    let narrow = measure(&editor, 2).unwrap();
    let wide = measure(&editor, 4).unwrap();

    assert_eq!(narrow.desired_height, std::num::NonZeroU16::new(3).unwrap());
    assert_eq!(wide.desired_height, std::num::NonZeroU16::new(2).unwrap());
}

// 폭 0은 임의 높이로 보정하지 않고 상위 레이아웃이 대기할 수 있는 구조화된 오류다.
#[test]
fn zero_width_is_an_explicit_measurement_error() {
    let editor = editor_with("\u{301}");

    let error = measure(&editor, 0).unwrap_err();

    assert_eq!(error, PromptMeasureError::ZeroWidth);
}

// 표시 불가능한 입력은 부정확한 높이를 반환하지 않고 원래 layout 원인을 보존한다.
#[test]
fn unrenderable_input_preserves_layout_failure() {
    let editor = editor_with("\u{301}");

    let error = measure(&editor, 4).unwrap_err();

    assert_eq!(
        error,
        PromptMeasureError::Layout(LayoutError::UnrenderableGrapheme {
            byte_index: 0,
            cause: crate::surface::GraphemeError::ZeroWidth,
        })
    );
}
