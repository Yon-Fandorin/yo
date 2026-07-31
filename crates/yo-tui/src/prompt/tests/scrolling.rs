use super::{
    PromptFrame, PromptViewState, editor_with, move_left, move_right, prompt_styles, render,
    rendered_text,
};
use crate::surface::{CellContent, Point, Rect, Size, Surface};

fn render_at(
    editor: &crate::input::editor::PromptEditor,
    state: &mut PromptViewState,
    size: Size,
) -> (PromptFrame, Surface) {
    let mut surface = Surface::new(size).unwrap();
    let frame = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(editor, &mut view, prompt_styles(), state).unwrap()
    };
    (frame, surface)
}

// 내용이 view보다 길면 마지막 행을 보이도록 내리고 숨겨진 위쪽 행은 그리지 않는다.
#[test]
fn scrolls_to_end_cursor_and_projects_only_visible_rows() {
    let editor = editor_with("abcdef");
    let mut state = PromptViewState::default();

    let (frame, surface) = render_at(&editor, &mut state, Size::new(2, 2));

    assert_eq!(
        frame,
        PromptFrame {
            cursor: Point::new(0, 1),
            content_height: std::num::NonZeroU16::new(4).unwrap(),
            first_visible_row: 2,
        }
    );
    assert_eq!(rendered_text(&surface, 0), "ef");
    assert_eq!(rendered_text(&surface, 1), "");
}

// 현재 커서가 이미 보이면 불필요하게 창을 움직이지 않는다.
#[test]
fn preserves_current_window_while_cursor_remains_visible() {
    let mut editor = editor_with("abcdefghi");
    let mut state = PromptViewState::default();
    let _ = render_at(&editor, &mut state, Size::new(2, 2));
    move_left(&mut editor, 1);

    let (frame, surface) = render_at(&editor, &mut state, Size::new(2, 2));

    assert_eq!(frame.first_visible_row, 3);
    assert_eq!(frame.cursor, Point::new(0, 1));
    assert_eq!(rendered_text(&surface, 0), "gh");
    assert_eq!(rendered_text(&surface, 1), "i");
}

// 커서가 현재 창 위로 이동하면 커서 행까지만 최소한으로 올린다.
#[test]
fn scrolls_up_only_until_cursor_is_visible() {
    let mut editor = editor_with("abcdef");
    let mut state = PromptViewState::default();
    let _ = render_at(&editor, &mut state, Size::new(2, 2));
    move_left(&mut editor, 3);

    let (frame, surface) = render_at(&editor, &mut state, Size::new(2, 2));

    assert_eq!(frame.first_visible_row, 1);
    assert_eq!(frame.cursor, Point::new(1, 0));
    assert_eq!(rendered_text(&surface, 0), "cd");
    assert_eq!(rendered_text(&surface, 1), "ef");
}

// 커서가 현재 창 바로 아래로 이동하면 마지막 행으로 점프하지 않고 한 행만 내린다.
#[test]
fn scrolls_down_only_until_cursor_is_visible() {
    let mut editor = editor_with("abcdefghi");
    move_left(&mut editor, 6);
    let mut state = PromptViewState::default();
    let _ = render_at(&editor, &mut state, Size::new(2, 2));
    move_right(&mut editor, 1);

    let (frame, surface) = render_at(&editor, &mut state, Size::new(2, 2));

    assert_eq!(frame.first_visible_row, 1);
    assert_eq!(frame.cursor, Point::new(0, 1));
    assert_eq!(rendered_text(&surface, 0), "cd");
    assert_eq!(rendered_text(&surface, 1), "ef");
}

// view가 커지면 이전 스크롤을 새 범위로 제한하면서 끝 커서를 계속 보인다.
#[test]
fn clamps_saved_scroll_after_view_grows() {
    let editor = editor_with("abc");
    let mut state = PromptViewState::default();
    let _ = render_at(&editor, &mut state, Size::new(1, 1));

    let (frame, surface) = render_at(&editor, &mut state, Size::new(1, 3));

    assert_eq!(frame.first_visible_row, 1);
    assert_eq!(frame.cursor, Point::new(0, 2));
    assert_eq!(rendered_text(&surface, 0), "b");
    assert_eq!(rendered_text(&surface, 1), "c");
}

// 모든 내용이 view에 들어오면 남아 있던 스크롤을 0으로 되돌린다.
#[test]
fn resets_saved_scroll_when_all_content_fits() {
    let editor = editor_with("abc");
    let mut state = PromptViewState::default();
    let _ = render_at(&editor, &mut state, Size::new(1, 1));

    let (frame, surface) = render_at(&editor, &mut state, Size::new(1, 4));

    assert_eq!(frame.first_visible_row, 0);
    assert_eq!(frame.cursor, Point::new(0, 3));
    assert_eq!(rendered_text(&surface, 0), "a");
}

// 장식된 폭 3 입력을 아래로 scroll하면 위·아래 rule은 유지하되 논리 첫 행의 marker는
// 숨기고, 보이는 본문과 cursor를 예약된 2칸 prefix 뒤에 정확히 투영한다.
#[test]
fn decorated_scroll_preserves_rules_and_prefix_without_repeating_marker() {
    let mut editor = editor_with("abc");
    move_left(&mut editor, 1);
    let mut state = PromptViewState::default();

    let (frame, surface) = render_at(&editor, &mut state, Size::new(3, 3));

    assert_eq!(
        frame,
        PromptFrame {
            cursor: Point::new(2, 1),
            content_height: std::num::NonZeroU16::new(3).unwrap(),
            first_visible_row: 2,
        }
    );
    assert_eq!(rendered_text(&surface, 0), "───");
    assert_eq!(rendered_text(&surface, 1), "c");
    assert_eq!(rendered_text(&surface, 2), "───");
    assert!(matches!(
        surface.cell(Point::new(0, 1)).unwrap().content(),
        CellContent::Blank
    ));
    assert!(matches!(
        surface.cell(Point::new(2, 1)).unwrap().content(),
        CellContent::Grapheme { text, .. } if text.as_ref() == "c"
    ));
}
