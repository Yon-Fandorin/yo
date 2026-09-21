use std::io::{self, Write};

use base64::{Engine as _, engine::general_purpose::STANDARD};

use super::{MAX_TEXT_BYTES, send};

// 한글·이모지·줄바꿈을 포함한 원문은 화면 줄바꿈이나 장식 없이 같은 UTF-8 바이트로 전송한다.
#[test]
fn osc52_carries_exact_source_text() {
    let source = "한글 🦀\n```rust\nfn main() {}\n```";
    let mut output = Vec::new();
    send(&mut output, source).unwrap();
    assert!(output.starts_with(b"\x1b]52;c;"));
    assert!(output.ends_with(b"\x07"));
    assert_eq!(
        STANDARD.decode(&output[7..output.len() - 1]).unwrap(),
        source.as_bytes()
    );
}

// 최대 인코딩 페이로드는 허용하고 첫 초과 바이트는 터미널에 아무것도 쓰지 않는다.
#[test]
fn first_excess_byte_is_rejected_before_output() {
    let mut output = Vec::new();
    send(&mut output, &"x".repeat(MAX_TEXT_BYTES)).unwrap();
    assert_eq!(output.len(), 7 + 100_000 + 1);
    output.clear();
    let error = send(&mut output, &"x".repeat(MAX_TEXT_BYTES + 1)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(output.is_empty());
}

struct FailingOutput {
    written: usize,
}

impl Write for FailingOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.written >= 9 {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let accepted = bytes.len().min(9 - self.written);
        self.written += accepted;
        Ok(accepted)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// 출력이 일부만 쓰이고 끊기면 성공으로 보고하지 않도록 오류를 전파한다.
#[test]
fn partial_write_failure_is_reported() {
    let mut output = FailingOutput { written: 0 };
    let error = send(&mut output, "answer").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(output.written, 9);
}

struct TransientFailure {
    bytes: Vec<u8>,
    fail_once: bool,
}

impl Write for TransientFailure {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len() >= 9 && self.fail_once {
            self.fail_once = false;
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let accepted = if self.fail_once {
            bytes.len().min(9 - self.bytes.len())
        } else {
            bytes.len()
        };
        self.bytes.extend_from_slice(&bytes[..accepted]);
        Ok(accepted)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// 일시적인 부분 쓰기 실패 뒤에는 OSC를 닫아 뒤따르는 터미널 복구 명령을 보호한다.
#[test]
fn transient_partial_write_closes_control_string_and_keeps_error() {
    let mut output = TransientFailure {
        bytes: Vec::new(),
        fail_once: true,
    };
    let error = send(&mut output, "answer").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(&output.bytes[..9], b"\x1b]52;c;YW");
    assert!(output.bytes.ends_with(b"\x1b\\"));
}
