use std::num::NonZeroU16;

use super::{encode, write};
use crate::{
    surface::{Point, Size, Style, Surface},
    terminal::{AnsiEncoder, TerminalOp},
};

// cursor 좌표는 u16 최댓값에서도 overflow 없이 ANSI의 1-based 좌표로 바뀐다.
#[test]
fn cursor_position_uses_checked_wider_parameters() {
    let mut encoder = AnsiEncoder::new(Vec::new());

    encoder
        .encode_operations(&[TerminalOp::MoveTo(Point::new(u16::MAX, u16::MAX))])
        .unwrap();

    assert_eq!(encoder.into_inner(), b"\x1b[65536;65536H");
}

// grapheme 원문 UTF-8은 재분할하거나 escape하지 않고 그대로 기록한다.
#[test]
fn grapheme_bytes_are_preserved() {
    let previous = Surface::new(Size::new(4, 1)).unwrap();
    let mut current = previous.clone();
    write(&mut current, Point::new(1, 0), "👩‍💻", Style::default());

    let bytes = encode(&previous, &current);

    assert_eq!(
        bytes,
        [b"\x1b[1;2H\x1b[0;39;49m".as_slice(), "👩‍💻".as_bytes()].concat()
    );
}

// Blank count는 cursor 제어 문자가 아닌 정확한 수의 ASCII space로 인코딩한다.
#[test]
fn blank_count_writes_exact_spaces() {
    let mut encoder = AnsiEncoder::new(Vec::new());

    encoder
        .encode_operations(&[TerminalOp::WriteBlank {
            count: NonZeroU16::new(513).unwrap(),
        }])
        .unwrap();

    assert_eq!(encoder.into_inner(), vec![b' '; 513]);
}

// typed operation 순서는 byte stream에서도 그대로 유지된다.
#[test]
fn operation_order_has_a_deterministic_byte_projection() {
    let mut encoder = AnsiEncoder::new(Vec::new());
    let style = Style::default();

    encoder
        .encode_operations(&[
            TerminalOp::MoveTo(Point::new(2, 3)),
            TerminalOp::SetStyle(style),
            TerminalOp::WriteGrapheme {
                text: "A",
                width: NonZeroU16::new(1).unwrap(),
            },
            TerminalOp::WriteBlank {
                count: NonZeroU16::new(2).unwrap(),
            },
        ])
        .unwrap();

    assert_eq!(encoder.into_inner(), b"\x1b[4;3H\x1b[0;39;49mA  ");
}
