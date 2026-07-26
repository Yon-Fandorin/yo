use crate::{
    surface::{Attributes, Color, Style},
    terminal::{AnsiEncoder, TerminalOp},
};

// terminal-default 전경·배경과 비어 있는 attribute를 완전한 reset sequence로 인코딩한다.
#[test]
fn default_style_is_explicit() {
    let mut encoder = AnsiEncoder::new(Vec::new());

    encoder
        .encode_operations(&[TerminalOp::SetStyle(Style::default())])
        .unwrap();

    assert_eq!(encoder.into_inner(), b"\x1b[0;39;49m");
}

// indexed color와 attribute는 고정된 parameter 순서로 결정론적으로 인코딩한다.
#[test]
fn indexed_style_has_stable_parameter_order() {
    let mut encoder = AnsiEncoder::new(Vec::new());
    let style = Style::new(
        Color::Indexed(7),
        Color::Indexed(201),
        Attributes::UNDERLINE.union(Attributes::BOLD),
    );

    encoder
        .encode_operations(&[TerminalOp::SetStyle(style)])
        .unwrap();

    assert_eq!(encoder.into_inner(), b"\x1b[0;38;5;7;48;5;201;1;4m");
}

// RGB color와 모든 지원 attribute가 손실 없이 ANSI SGR parameter로 투영된다.
#[test]
fn rgb_style_encodes_every_supported_attribute() {
    let mut encoder = AnsiEncoder::new(Vec::new());
    let attributes = [
        Attributes::BOLD,
        Attributes::DIM,
        Attributes::ITALIC,
        Attributes::UNDERLINE,
        Attributes::BLINK,
        Attributes::REVERSE,
        Attributes::HIDDEN,
        Attributes::STRIKETHROUGH,
    ]
    .into_iter()
    .fold(Attributes::empty(), Attributes::union);
    let style = Style::new(
        Color::Rgb {
            red: 1,
            green: 2,
            blue: 3,
        },
        Color::Rgb {
            red: 250,
            green: 251,
            blue: 252,
        },
        attributes,
    );

    encoder
        .encode_operations(&[TerminalOp::SetStyle(style)])
        .unwrap();

    assert_eq!(
        encoder.into_inner(),
        b"\x1b[0;38;2;1;2;3;48;2;250;251;252;1;2;3;4;5;7;8;9m"
    );
}
