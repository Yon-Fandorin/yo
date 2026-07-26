use std::num::NonZeroU16;

use yo_tui::{
    surface::{Attributes, Color, Point, Style},
    terminal::{AnsiEncoder, TerminalOp},
};

use super::support::{fixture_ops, fixture_surface};

// 하나의 completed Surface가 예상한 typed terminal 의미를 정확히 만든다.
#[test]
fn terminal_operations_match_the_shared_surface_fixture() {
    let surface = fixture_surface();
    let operations = fixture_ops(&surface);

    assert_eq!(
        operations.as_slice(),
        &[
            TerminalOp::MoveTo(Point::new(0, 0)),
            TerminalOp::SetStyle(Style::new(
                Color::Rgb {
                    red: 1,
                    green: 2,
                    blue: 3,
                },
                Color::Indexed(42),
                Attributes::BOLD.union(Attributes::UNDERLINE),
            )),
            TerminalOp::WriteGrapheme {
                text: "A",
                width: nonzero(1),
            },
            TerminalOp::SetStyle(Style::new(
                Color::Indexed(15),
                Color::Indexed(4),
                Attributes::empty(),
            )),
            TerminalOp::WriteGrapheme {
                text: "가",
                width: nonzero(2),
            },
            TerminalOp::SetStyle(Style::new(
                Color::Default,
                Color::Indexed(1),
                Attributes::empty(),
            )),
            TerminalOp::WriteBlank { count: nonzero(1) },
            TerminalOp::SetStyle(Style::new(
                Color::Rgb {
                    red: 200,
                    green: 150,
                    blue: 100,
                },
                Color::Default,
                Attributes::DIM.union(Attributes::ITALIC),
            )),
            TerminalOp::WriteGrapheme {
                text: "👩‍💻",
                width: nonzero(2),
            },
            TerminalOp::MoveTo(Point::new(0, 1)),
            TerminalOp::SetStyle(Style::new(
                Color::Indexed(1),
                Color::Indexed(2),
                Attributes::REVERSE.union(Attributes::STRIKETHROUGH),
            )),
            TerminalOp::WriteGrapheme {
                text: "<",
                width: nonzero(1),
            },
            TerminalOp::SetStyle(Style::new(
                Color::Default,
                Color::Default,
                Attributes::BLINK.union(Attributes::HIDDEN),
            )),
            TerminalOp::WriteGrapheme {
                text: "&",
                width: nonzero(1),
            },
        ]
    );
}

// 동일 Surface의 ANSI bytes를 사람이 읽을 수 있는 escaped golden으로 고정한다.
#[test]
fn ansi_matches_the_shared_surface_fixture() {
    let surface = fixture_surface();
    let operations = fixture_ops(&surface);
    let mut encoder = AnsiEncoder::new(Vec::new());
    encoder.encode(&operations).unwrap();

    assert_eq!(
        escape_ansi(&encoder.into_inner()),
        include_str!("../fixtures/rendering-parity/expected.ansi.txt").trim_end()
    );
}

fn nonzero(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value).unwrap()
}

fn escape_ansi(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace('\u{1b}', "\\x1b")
        .replace(' ', "\\x20")
}
