//! 사용자가 명시적으로 요청한 텍스트의 제한된 OSC 52 전송.

use std::io::{self, Write};

use base64::{Engine as _, engine::general_purpose::STANDARD};

/// Base64 페이로드를 100,000바이트 이내로 유지하는 UTF-8 원문 한도.
pub(crate) const MAX_TEXT_BYTES: usize = 75_000;

pub(crate) fn send(output: &mut impl Write, text: &str) -> io::Result<()> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "assistant answer exceeds terminal clipboard limit",
        ));
    }
    let encoded = STANDARD.encode(text.as_bytes());
    let mut sequence = Vec::with_capacity(7 + encoded.len() + 1);
    sequence.extend_from_slice(b"\x1b]52;c;");
    sequence.extend_from_slice(encoded.as_bytes());
    sequence.push(0x07);
    if let Err(error) = output.write_all(&sequence) {
        // Fullscreen 정리는 터미널 모드만 복원하므로, 부분 OSC가 뒤따르는 복원 시퀀스를
        // 삼키지 않도록 제어 문자열을 먼저 닫는다.
        let _ = output.write_all(b"\x1b\\");
        let _ = output.flush();
        return Err(error);
    }
    output.flush()
}

#[cfg(test)]
mod tests;
