use std::io::{self, Write};

use super::write;
use crate::{
    surface::{FrameDiff, Point, Size, Style, Surface},
    terminal::{AnsiEncodeError, AnsiEncoder, TerminalOp, TerminalOps},
};

// resize signal은 byte를 하나도 쓰기 전에 mode controller로 돌려보낸다.
#[test]
fn frame_size_change_fails_before_output() {
    let previous = Surface::new(Size::new(2, 1)).unwrap();
    let current = Surface::new(Size::new(3, 1)).unwrap();
    let diff = FrameDiff::between(&previous, &current);
    let operations = TerminalOps::from_diff(&diff);
    let mut encoder = AnsiEncoder::new(Vec::new());

    let error = encoder.encode(&operations).unwrap_err();

    assert!(matches!(
        error,
        AnsiEncodeError::FrameSizeChanged {
            previous: Size {
                width: 2,
                height: 1
            },
            current: Size {
                width: 3,
                height: 1
            }
        }
    ));
    assert!(encoder.into_inner().is_empty());
}

// resize signal이 operation 중간에 있어도 preflight가 partial output을 막는다.
#[test]
fn misplaced_frame_size_change_also_fails_before_output() {
    let mut encoder = AnsiEncoder::new(Vec::new());

    let error = encoder
        .encode_operations(&[
            TerminalOp::MoveTo(Point::new(1, 1)),
            TerminalOp::FrameSizeChanged {
                previous: Size::new(2, 2),
                current: Size::new(3, 3),
            },
        ])
        .unwrap_err();

    assert!(matches!(error, AnsiEncodeError::FrameSizeChanged { .. }));
    assert!(encoder.into_inner().is_empty());
}

// underlying writer 실패는 원래 io::Error를 source로 보존한다.
#[test]
fn writer_failure_is_reported_without_masking_its_source() {
    let previous = Surface::new(Size::new(2, 1)).unwrap();
    let mut current = previous.clone();
    write(&mut current, Point::new(0, 0), "A", Style::default());
    let diff = FrameDiff::between(&previous, &current);
    let operations = TerminalOps::from_diff(&diff);
    let mut encoder = AnsiEncoder::new(FailingWriter);

    let error = encoder.encode(&operations).unwrap_err();

    assert!(matches!(error, AnsiEncodeError::Io(_)));
    assert_eq!(
        std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<io::Error>())
            .map(io::Error::kind),
        Some(io::ErrorKind::BrokenPipe)
    );
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "test writer"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
