use std::io;

use super::super::write_attention_bell;

// terminal writer는 bell 문자를 escape sequence로 확장하거나 다른 byte를 덧붙이지 않습니다.
#[test]
fn attention_bell_writes_exactly_one_bel_byte() {
    let mut output = Vec::new();

    write_attention_bell(&mut output).expect("the in-memory terminal writer accepts BEL");

    assert_eq!(output, [0x07]);
}

struct FailingWriter;

impl io::Write for FailingWriter {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "test failure"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// bell writer 오류는 live loop의 주 오류를 바꾸지 않도록 호출자가 처리할 수 있습니다.
#[test]
fn attention_bell_write_failure_is_reported_without_panicking() {
    let error = write_attention_bell(&mut FailingWriter).expect_err("writer must fail");

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
}
