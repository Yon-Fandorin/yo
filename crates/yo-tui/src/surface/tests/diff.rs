use super::super::{
    CellContent, Color, FrameDiff, Grapheme, Point, Rect, Size, Style, Surface, WriteOutcome,
};

fn write(surface: &mut Surface, point: Point, text: &str, style: Style) {
    let size = surface.size();
    let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
    assert_eq!(
        view.write(point, Grapheme::try_from(text).unwrap(), style),
        WriteOutcome::Written
    );
}

// 같은 크기와 셀 상태를 가진 두 completed frame은 변경 span을 만들지 않는다.
#[test]
fn identical_frames_have_an_empty_diff() {
    let previous = Surface::new(Size::new(4, 2)).unwrap();
    let current = previous.clone();

    let diff = FrameDiff::between(&previous, &current);

    assert!(diff.is_empty());
    assert_eq!(diff.previous_size(), Size::new(4, 2));
    assert_eq!(diff.current_size(), Size::new(4, 2));
}

// adapter가 기존 출력 위치를 신뢰하지 못하면 같은 frame도 모든 행을 다시 그릴 수 있어야 한다.
#[test]
fn complete_diff_emits_every_row_for_an_identical_frame() {
    let current = Surface::new(Size::new(3, 2)).unwrap();

    let diff = FrameDiff::complete(current.size(), &current);

    assert_eq!(diff.previous_size(), Size::new(3, 2));
    assert_eq!(diff.current_size(), Size::new(3, 2));
    assert_eq!(
        diff.spans()
            .iter()
            .map(|span| (span.row(), span.start_column(), span.end_column()))
            .collect::<Vec<_>>(),
        [(0, 0, 3), (1, 0, 3)]
    );
}

// 폭이 0인 full redraw도 크기 정보는 유지하고 존재하지 않는 cell span은 만들지 않는다.
#[test]
fn complete_diff_preserves_zero_width_geometry() {
    let current = Surface::new(Size::new(0, 2)).unwrap();

    let diff = FrameDiff::complete(Size::new(4, 2), &current);

    assert_eq!(diff.previous_size(), Size::new(4, 2));
    assert_eq!(diff.current_size(), Size::new(0, 2));
    assert!(diff.spans().is_empty());
}

// 서로 떨어진 변경은 row와 column 오름차순의 독립 span으로 방출한다.
#[test]
fn changed_spans_have_stable_row_and_column_order() {
    let previous = Surface::new(Size::new(6, 3)).unwrap();
    let mut current = previous.clone();
    write(&mut current, Point::new(4, 0), "A", Style::default());
    write(&mut current, Point::new(1, 0), "B", Style::default());
    write(&mut current, Point::new(2, 2), "C", Style::default());

    let diff = FrameDiff::between(&previous, &current);
    let positions = diff
        .spans()
        .iter()
        .map(|span| (span.row(), span.start_column(), span.end_column()))
        .collect::<Vec<_>>();

    assert_eq!(positions, [(0, 1, 2), (0, 4, 5), (2, 2, 3)]);
}

// 같은 폭의 wide grapheme 문자열만 바뀌어도 continuation까지 완전한 span에 포함한다.
#[test]
fn leader_change_preserves_the_complete_current_grapheme() {
    let size = Size::new(4, 1);
    let mut previous = Surface::new(size).unwrap();
    let mut current = Surface::new(size).unwrap();
    write(&mut previous, Point::new(1, 0), "가", Style::default());
    write(&mut current, Point::new(1, 0), "나", Style::default());

    let diff = FrameDiff::between(&previous, &current);
    let [span] = diff.spans() else {
        panic!("one complete grapheme span is expected");
    };

    assert_eq!((span.start_column(), span.end_column()), (1, 3));
    assert!(matches!(
        span.cells()[0].content(),
        CellContent::Grapheme { text, width }
            if text.as_ref() == "나" && width.get() == 2
    ));
    assert!(matches!(
        span.cells()[1].content(),
        CellContent::Continuation { back } if back.get() == 1
    ));
}

// previous의 wide grapheme이 좁아지면 이전 continuation까지 변경 span에 남긴다.
#[test]
fn narrower_current_grapheme_covers_the_previous_wide_footprint() {
    let size = Size::new(4, 1);
    let mut previous = Surface::new(size).unwrap();
    let mut current = Surface::new(size).unwrap();
    write(&mut previous, Point::new(0, 0), "가", Style::default());
    write(&mut current, Point::new(0, 0), "A", Style::default());

    let diff = FrameDiff::between(&previous, &current);
    let [span] = diff.spans() else {
        panic!("the complete previous footprint is expected");
    };

    assert_eq!((span.start_column(), span.end_column()), (0, 2));
    assert!(matches!(
        span.cells()[0].content(),
        CellContent::Grapheme { .. }
    ));
    assert_eq!(span.cells()[1].content(), &CellContent::Blank);
}

// span의 resolved current cell만 적용해도 previous frame을 current frame으로 재구성할 수 있다.
#[test]
fn spans_contain_enough_state_to_reconstruct_the_current_frame() {
    let size = Size::new(7, 2);
    let mut previous = Surface::new(size).unwrap();
    write(&mut previous, Point::new(1, 0), "가", Style::default());
    let mut current = previous.clone();
    write(
        &mut current,
        Point::new(1, 0),
        "A",
        Style {
            foreground: Color::Indexed(2),
            ..Style::default()
        },
    );
    write(&mut current, Point::new(5, 1), "👩‍💻", Style::default());

    let diff = FrameDiff::between(&previous, &current);
    let mut reconstructed = previous.clone();
    for span in diff.spans() {
        for (offset, cell) in span.cells().iter().cloned().enumerate() {
            let column = usize::from(span.start_column()) + offset;
            let index = usize::from(span.row()) * usize::from(size.width) + column;
            reconstructed.replace_by_index(index, cell);
        }
    }

    assert_eq!(reconstructed, current);
}

// 크기가 달라지면 current 크기를 기록하고 현재 frame의 모든 non-empty row를 방출한다.
#[test]
fn size_change_emits_complete_current_rows() {
    let previous = Surface::new(Size::new(2, 1)).unwrap();
    let current = Surface::new(Size::new(3, 2)).unwrap();

    let diff = FrameDiff::between(&previous, &current);

    assert!(!diff.is_empty());
    assert_eq!(diff.previous_size(), Size::new(2, 1));
    assert_eq!(diff.current_size(), Size::new(3, 2));
    assert_eq!(
        diff.spans()
            .iter()
            .map(|span| (span.row(), span.start_column(), span.end_column()))
            .collect::<Vec<_>>(),
        [(0, 0, 3), (1, 0, 3)]
    );
}

// frame이 줄어들어도 previous/current 크기와 남은 current row 전체를 함께 전달한다.
#[test]
fn shrinking_frame_preserves_resize_metadata_and_current_rows() {
    let previous = Surface::new(Size::new(5, 3)).unwrap();
    let current = Surface::new(Size::new(2, 1)).unwrap();

    let diff = FrameDiff::between(&previous, &current);

    assert_eq!(diff.previous_size(), Size::new(5, 3));
    assert_eq!(diff.current_size(), Size::new(2, 1));
    let [span] = diff.spans() else {
        panic!("the remaining current row is expected");
    };
    assert_eq!(
        (span.row(), span.start_column(), span.end_column()),
        (0, 0, 2)
    );
}

// width가 0인 current frame도 size change를 잃지 않고 빈 span 집합으로 표현한다.
#[test]
fn zero_width_size_change_is_not_an_empty_diff() {
    let previous = Surface::new(Size::new(1, 1)).unwrap();
    let current = Surface::new(Size::new(0, 2)).unwrap();

    let diff = FrameDiff::between(&previous, &current);

    assert!(!diff.is_empty());
    assert!(diff.spans().is_empty());
    assert_eq!(diff.current_size(), Size::new(0, 2));
}

// height가 0인 current frame도 이전 크기와 resize 사실을 보존한다.
#[test]
fn zero_height_size_change_is_not_an_empty_diff() {
    let previous = Surface::new(Size::new(2, 2)).unwrap();
    let current = Surface::new(Size::new(2, 0)).unwrap();

    let diff = FrameDiff::between(&previous, &current);

    assert!(!diff.is_empty());
    assert!(diff.spans().is_empty());
    assert_eq!(diff.previous_size(), Size::new(2, 2));
    assert_eq!(diff.current_size(), Size::new(2, 0));
}
