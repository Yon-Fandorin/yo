use std::io::{IsTerminal, Read};

use crate::diagnostic::AppError;

pub(crate) fn read_input(prompt: Option<String>) -> Result<String, AppError> {
    let stdin = std::io::stdin();
    let is_terminal = stdin.is_terminal();
    let mut stdin = stdin.lock();
    read_input_from(prompt, &mut stdin, is_terminal)
}

fn read_input_from(
    prompt: Option<String>,
    stdin: &mut impl Read,
    stdin_is_terminal: bool,
) -> Result<String, AppError> {
    let stdin_text = if stdin_is_terminal {
        String::new()
    } else {
        let mut bytes = Vec::new();
        stdin
            .read_to_end(&mut bytes)
            .map_err(|error| AppError::single("reading print input from stdin", error))?;
        String::from_utf8(bytes).map_err(|error| {
            AppError::single("reading UTF-8 print input from stdin", error.utf8_error())
        })?
    };
    compose_input(stdin_text, prompt)
}

fn compose_input(stdin: String, prompt: Option<String>) -> Result<String, AppError> {
    let prompt = prompt.unwrap_or_default();
    let input = match (stdin.is_empty(), prompt.is_empty()) {
        (true, true) => {
            return Err(AppError::message(
                "print mode requires a positional prompt or non-empty piped stdin",
            ));
        },
        (false, true) => stdin,
        (true, false) => prompt,
        (false, false) if stdin.ends_with('\n') => stdin + &prompt,
        (false, false) => stdin + "\n" + &prompt,
    };
    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    // positional prompt만 있는 TTY, stdin만 있는 pipe, 둘을 함께 쓰는 경우의 순서와 LF
    // 경계를 정확히 보존해 한 Submission 텍스트를 만듭니다.
    #[test]
    fn input_composition_is_stdin_first_and_lf_stable() {
        assert_eq!(
            read_input_from(Some("prompt".to_owned()), &mut &b""[..], true).unwrap(),
            "prompt"
        );
        assert_eq!(
            read_input_from(None, &mut &b"stdin"[..], false).unwrap(),
            "stdin"
        );
        assert_eq!(
            read_input_from(Some("prompt".to_owned()), &mut &b"stdin"[..], false).unwrap(),
            "stdin\nprompt"
        );
        assert_eq!(
            read_input_from(Some("prompt".to_owned()), &mut &b"stdin\n"[..], false).unwrap(),
            "stdin\nprompt"
        );
    }

    // 입력이 없거나 pipe가 UTF-8이 아니면 Session이나 Backend를 만들기 전에 명확히
    // 실패하여 비어 있거나 손상된 Submission을 추측하지 않습니다.
    #[test]
    fn invalid_or_empty_input_fails_before_startup() {
        assert!(read_input_from(None, &mut &b""[..], true).is_err());
        assert!(read_input_from(None, &mut &b""[..], false).is_err());
        assert!(read_input_from(None, &mut &[0xff][..], false).is_err());
    }
}
