use std::num::NonZeroU16;

use crate::{
    surface::{
        Attributes, Color, FrameDiff, Grapheme, Point, Rect, Size, Style, Surface, WriteOutcome,
    },
    terminal::{TerminalOp, TerminalOps},
};

fn style(index: u8) -> Style {
    Style::new(Color::Indexed(index), Color::Default, Attributes::BOLD)
}

fn write(surface: &mut Surface, point: Point, text: &str, style: Style) {
    let size = surface.size();
    let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
    assert_eq!(
        view.write(point, Grapheme::try_from(text).unwrap(), style),
        WriteOutcome::Written
    );
}

// 변경이 없는 frame은 terminal side effect도 만들지 않는다.
#[test]
fn empty_diff_compiles_to_no_operations() {
    let previous = Surface::new(Size::new(3, 1)).unwrap();
    let current = previous.clone();
    let diff = FrameDiff::between(&previous, &current);

    let operations = TerminalOps::from_diff(&diff);

    assert!(operations.is_empty());
}

// 크기 변화는 span 유무와 관계없이 mode controller가 처리할 typed signal로 보존한다.
#[test]
fn size_change_is_the_first_typed_operation() {
    let previous = Surface::new(Size::new(2, 1)).unwrap();
    let current = Surface::new(Size::new(3, 1)).unwrap();
    let diff = FrameDiff::between(&previous, &current);

    let operations = TerminalOps::from_diff(&diff);

    assert_eq!(
        operations.as_slice(),
        [
            TerminalOp::FrameSizeChanged {
                previous: Size::new(2, 1),
                current: Size::new(3, 1),
            },
            TerminalOp::MoveTo(Point::new(0, 0)),
            TerminalOp::SetStyle(Style::default()),
            TerminalOp::WriteBlank {
                count: NonZeroU16::new(3).unwrap(),
            },
        ]
    );
}

// current frame의 차원이 0이어도 resize 의미가 빈 operation으로 소실되지 않는다.
#[test]
fn zero_dimension_size_change_remains_observable() {
    for current_size in [Size::new(0, 2), Size::new(2, 0)] {
        let previous = Surface::new(Size::new(2, 2)).unwrap();
        let current = Surface::new(current_size).unwrap();
        let diff = FrameDiff::between(&previous, &current);

        let operations = TerminalOps::from_diff(&diff);

        assert!(!operations.is_empty());
        assert_eq!(
            operations.as_slice(),
            [TerminalOp::FrameSizeChanged {
                previous: Size::new(2, 2),
                current: current_size,
            }]
        );
    }
}

// 하나의 grapheme 변경을 cursor 이동, resolved style, grapheme write 순서로 표현한다.
#[test]
fn grapheme_change_has_an_explicit_typed_order() {
    let previous = Surface::new(Size::new(3, 1)).unwrap();
    let mut current = previous.clone();
    let selected = style(1);
    write(&mut current, Point::new(1, 0), "A", selected);
    let diff = FrameDiff::between(&previous, &current);

    let operations = TerminalOps::from_diff(&diff);

    assert_eq!(
        operations.as_slice(),
        [
            TerminalOp::MoveTo(Point::new(1, 0)),
            TerminalOp::SetStyle(selected),
            TerminalOp::WriteGrapheme {
                text: "A",
                width: NonZeroU16::new(1).unwrap(),
            },
        ]
    );
}

// wide grapheme은 continuation을 별도 write하지 않고 확정된 폭과 원문을 한 번만 전달한다.
#[test]
fn wide_grapheme_is_one_write_operation() {
    let previous = Surface::new(Size::new(4, 1)).unwrap();
    let mut current = previous.clone();
    write(&mut current, Point::new(1, 0), "가", style(2));
    let diff = FrameDiff::between(&previous, &current);

    let operations = TerminalOps::from_diff(&diff);

    assert!(matches!(
        operations.as_slice(),
        [
            TerminalOp::MoveTo(Point { x: 1, y: 0 }),
            TerminalOp::SetStyle(_),
            TerminalOp::WriteGrapheme { text: "가", width }
        ] if width.get() == 2
    ));
}

// wide grapheme을 좁게 덮고 남은 footprint는 같은 incoming style의 blank write가 된다.
#[test]
fn narrower_overwrite_emits_the_cleanup_blank() {
    let size = Size::new(4, 1);
    let mut previous = Surface::new(size).unwrap();
    write(&mut previous, Point::new(0, 0), "가", style(1));
    let mut current = previous.clone();
    let incoming = style(3);
    write(&mut current, Point::new(0, 0), "A", incoming);
    let diff = FrameDiff::between(&previous, &current);

    let operations = TerminalOps::from_diff(&diff);

    assert_eq!(
        operations.as_slice(),
        [
            TerminalOp::MoveTo(Point::new(0, 0)),
            TerminalOp::SetStyle(incoming),
            TerminalOp::WriteGrapheme {
                text: "A",
                width: NonZeroU16::new(1).unwrap(),
            },
            TerminalOp::WriteBlank {
                count: NonZeroU16::new(1).unwrap(),
            },
        ]
    );
}

// 인접 셀의 resolved style이 바뀌면 byte encoding 전에 명시적인 SetStyle 경계를 만든다.
#[test]
fn adjacent_style_changes_are_explicit() {
    let previous = Surface::new(Size::new(3, 1)).unwrap();
    let mut current = previous.clone();
    write(&mut current, Point::new(0, 0), "A", style(4));
    write(&mut current, Point::new(1, 0), "B", style(5));
    let diff = FrameDiff::between(&previous, &current);

    let operations = TerminalOps::from_diff(&diff);

    assert_eq!(
        operations.as_slice(),
        [
            TerminalOp::MoveTo(Point::new(0, 0)),
            TerminalOp::SetStyle(style(4)),
            TerminalOp::WriteGrapheme {
                text: "A",
                width: NonZeroU16::new(1).unwrap(),
            },
            TerminalOp::SetStyle(style(5)),
            TerminalOp::WriteGrapheme {
                text: "B",
                width: NonZeroU16::new(1).unwrap(),
            },
        ]
    );
}

// 떨어진 span도 같은 style이면 terminal program 안에서 불필요한 style 재선택을 하지 않는다.
#[test]
fn selected_style_is_reused_across_spans() {
    let previous = Surface::new(Size::new(4, 2)).unwrap();
    let mut current = previous.clone();
    let shared = style(6);
    write(&mut current, Point::new(0, 0), "A", shared);
    write(&mut current, Point::new(2, 1), "B", shared);
    let diff = FrameDiff::between(&previous, &current);

    let operations = TerminalOps::from_diff(&diff);

    assert_eq!(
        operations.as_slice(),
        [
            TerminalOp::MoveTo(Point::new(0, 0)),
            TerminalOp::SetStyle(shared),
            TerminalOp::WriteGrapheme {
                text: "A",
                width: NonZeroU16::new(1).unwrap(),
            },
            TerminalOp::MoveTo(Point::new(2, 1)),
            TerminalOp::WriteGrapheme {
                text: "B",
                width: NonZeroU16::new(1).unwrap(),
            },
        ]
    );
}

// 같은 style의 연속 Blank는 하나의 count를 가진 typed operation으로 묶는다.
#[test]
fn adjacent_blanks_with_the_same_style_are_coalesced() {
    let size = Size::new(4, 1);
    let previous = Surface::new(size).unwrap();
    let mut current = previous.clone();
    let cleared = style(7);
    assert_eq!(
        current
            .view(Rect::new(Point::new(0, 0), size))
            .unwrap()
            .clear(cleared),
        WriteOutcome::Written
    );
    let diff = FrameDiff::between(&previous, &current);

    let operations = TerminalOps::from_diff(&diff);

    assert_eq!(
        operations.as_slice(),
        [
            TerminalOp::MoveTo(Point::new(0, 0)),
            TerminalOp::SetStyle(cleared),
            TerminalOp::WriteBlank {
                count: NonZeroU16::new(4).unwrap(),
            },
        ]
    );
}
