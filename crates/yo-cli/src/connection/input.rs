use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
};

use nix::sys::termios::{self, LocalFlags, SetArg, Termios};
use yo_core::ApiCredential;

use super::presentation::{Confirmation, PresentationStyle, default_width};
use crate::AppError;

const MAX_INPUT_BYTES: usize = 16 * 1024;

pub(super) trait ExternalConnectInput {
    fn confirm(&mut self, preview: &Confirmation) -> Result<bool, AppError>;
    fn read_credential(&mut self, account: &str) -> Result<ApiCredential, AppError>;
}

pub(super) trait ExternalDisconnectInput {
    fn select_target(&mut self, choices: &[String]) -> Result<String, AppError>;
    fn confirm(&mut self, preview: &Confirmation) -> Result<bool, AppError>;
}

pub(super) struct TtyConnectionInput {
    terminal: Option<File>,
    style: PresentationStyle,
}

impl TtyConnectionInput {
    pub(super) fn new() -> Self {
        Self {
            terminal: None,
            style: PresentationStyle::for_controlling_terminal(),
        }
    }

    fn terminal(&mut self) -> Result<&mut File, AppError> {
        if self.terminal.is_none() {
            let terminal = OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/tty")
                .map_err(|error| AppError::single("opening the controlling terminal", error))?;
            self.terminal = Some(terminal);
        }
        Ok(self
            .terminal
            .as_mut()
            .expect("the controlling terminal was opened above"))
    }

    fn read_line(mut terminal: &File) -> Result<String, AppError> {
        let mut bytes = Vec::new();
        let mut byte = [0_u8; 1];
        while bytes.len() <= MAX_INPUT_BYTES {
            let count = terminal
                .read(&mut byte)
                .map_err(|error| AppError::single("reading the controlling terminal", error))?;
            if count == 0 || byte[0] == b'\n' {
                break;
            }
            if byte[0] != b'\r' {
                bytes.push(byte[0]);
            }
        }
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(AppError::message(format!(
                "terminal input exceeds the {MAX_INPUT_BYTES}-byte limit"
            )));
        }
        String::from_utf8(bytes)
            .map_err(|_| AppError::message("terminal input must be valid UTF-8"))
    }
}

impl ExternalConnectInput for TtyConnectionInput {
    fn confirm(&mut self, preview: &Confirmation) -> Result<bool, AppError> {
        let style = self.style;
        let terminal = self.terminal()?;
        let width = rustix::termios::tcgetwinsize(&*terminal)
            .ok()
            .and_then(|size| std::num::NonZeroU16::new(size.ws_col))
            .unwrap_or_else(default_width);
        let rendered = preview
            .render_styled(width, style)
            .map_err(|error| AppError::single("formatting the connection confirmation", error))?;
        write!(terminal, "{rendered}\n\n{}", preview.prompt())
            .and_then(|()| terminal.flush())
            .map_err(|error| AppError::single("writing the connection confirmation", error))?;
        let answer = Self::read_line(terminal)?;
        Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "YES" | "Yes"))
    }

    fn read_credential(&mut self, account: &str) -> Result<ApiCredential, AppError> {
        let terminal = self.terminal()?;
        write!(terminal, "API key for {account}: ")
            .and_then(|()| terminal.flush())
            .map_err(|error| AppError::single("writing the API-key prompt", error))?;
        let original = termios::tcgetattr(&*terminal)
            .map_err(|error| AppError::single("reading terminal echo settings", error))?;
        let mut hidden = original.clone();
        hidden.local_flags.remove(LocalFlags::ECHO);
        termios::tcsetattr(&*terminal, SetArg::TCSAFLUSH, &hidden)
            .map_err(|error| AppError::single("disabling terminal echo", error))?;
        let restore = EchoRestore {
            terminal,
            original: Some(original),
        };
        let value = Self::read_line(terminal);
        let restore_result = restore.restore();
        writeln!(terminal)
            .and_then(|()| terminal.flush())
            .map_err(|error| AppError::single("finishing the API-key prompt", error))?;
        restore_result?;
        ApiCredential::new(value?).map_err(|error| AppError::single("reading the API key", error))
    }
}

impl ExternalDisconnectInput for TtyConnectionInput {
    fn select_target(&mut self, choices: &[String]) -> Result<String, AppError> {
        let terminal = self.terminal()?;
        write!(
            terminal,
            "Select one managed target by entering its exact reference:\n  - {}\nTarget: ",
            choices.join("\n  - ")
        )
        .and_then(|()| terminal.flush())
        .map_err(|error| AppError::single("writing the disconnect target prompt", error))?;
        Self::read_line(terminal)
    }

    fn confirm(&mut self, preview: &Confirmation) -> Result<bool, AppError> {
        <Self as ExternalConnectInput>::confirm(self, preview)
    }
}

struct EchoRestore<'a> {
    terminal: &'a File,
    original: Option<Termios>,
}

impl EchoRestore<'_> {
    fn restore(self) -> Result<(), AppError> {
        self.restore_with(|terminal, original| {
            termios::tcsetattr(terminal, SetArg::TCSAFLUSH, original)
        })
    }

    fn restore_with(
        mut self,
        restore: impl FnOnce(&File, &Termios) -> Result<(), nix::errno::Errno>,
    ) -> Result<(), AppError> {
        let original = self
            .original
            .as_ref()
            .expect("terminal restoration is attempted exactly once");
        restore(self.terminal, original)
            .map_err(|error| AppError::single("restoring terminal echo settings", error))?;
        self.original.take();
        Ok(())
    }
}

impl Drop for EchoRestore<'_> {
    fn drop(&mut self) {
        if let Some(original) = self.original.as_ref() {
            let _ = termios::tcsetattr(self.terminal, SetArg::TCSAFLUSH, original);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        io::Read,
        thread,
        time::{Duration, Instant},
    };

    use nix::{
        fcntl::{FcntlArg, OFlag, fcntl},
        pty::openpty,
        sys::termios::{LocalFlags, tcgetattr},
        unistd::write,
    };
    use rustix::termios::{Winsize, tcsetwinsize};
    use yo_core::CompleteModelBinding;
    use yo_tui::surface::cell_width;

    use super::{
        super::presentation::{BindingDetails, ConnectPreview},
        *,
    };

    // 실제 terminal처럼 PTY master가 prompt를 먼저 소비한 뒤 API key 입력 동안 ECHO가
    // 꺼지고, master에 secret이 나타나지 않으며 성공 뒤 termios가 정확히 복구됩니다.
    #[test]
    fn credential_input_hides_secret_and_restores_exact_termios() {
        let pty = openpty(None, None).unwrap();
        let observed_slave = pty.slave.try_clone().unwrap();
        let original = tcgetattr(&observed_slave).unwrap();
        let child = thread::spawn(move || {
            let mut input = TtyConnectionInput {
                terminal: Some(File::from(pty.slave)),
                style: PresentationStyle::Plain,
            };
            input.read_credential("vendor:team").unwrap()
        });

        let mut master = File::from(pty.master);
        fcntl(&master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
        let expected_prompt = b"API key for vendor:team: ";
        let mut output = vec![0_u8; expected_prompt.len()];
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut prompt_bytes = 0;
        while prompt_bytes < output.len() {
            match master.read(&mut output[prompt_bytes..]) {
                Ok(0) => panic!("credential prompt closed its PTY before completing"),
                Ok(count) => prompt_bytes += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "credential prompt did not complete"
                    );
                    thread::yield_now();
                },
                Err(error) => panic!("reading credential prompt failed: {error}"),
            }
        }
        assert_eq!(output, expected_prompt);

        while tcgetattr(&observed_slave)
            .unwrap()
            .local_flags
            .contains(LocalFlags::ECHO)
        {
            assert!(
                Instant::now() < deadline,
                "credential prompt did not disable echo"
            );
            thread::yield_now();
        }
        write(&master, b"sentinel-secret\n").unwrap();
        assert_eq!(child.join().unwrap().expose_secret(), "sentinel-secret");
        assert_eq!(tcgetattr(&observed_slave).unwrap(), original);

        let mut buffer = [0_u8; 256];
        loop {
            match master.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => output.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("reading PTY output failed: {error}"),
            }
        }
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("API key for vendor:team:"));
        assert!(!output.contains("sentinel-secret"));
    }

    // 명시적 termios 복구가 한 번 실패해도 original 설정을 guard에 남겨 Drop이 즉시
    // 재시도하며, 실제 PTY가 echo-disabled 상태로 유출되지 않고 호출 전 설정으로 돌아옵니다.
    #[test]
    fn failed_explicit_restore_keeps_original_for_drop_retry() {
        let pty = openpty(None, None).unwrap();
        let terminal = File::from(pty.slave);
        let original = tcgetattr(&terminal).unwrap();
        let mut hidden = original.clone();
        hidden.local_flags.remove(LocalFlags::ECHO);
        termios::tcsetattr(&terminal, SetArg::TCSAFLUSH, &hidden).unwrap();

        let error = EchoRestore {
            terminal: &terminal,
            original: Some(original.clone()),
        }
        .restore_with(|_, _| Err(nix::errno::Errno::EIO))
        .expect_err("the injected explicit restore must fail");

        assert!(
            error
                .to_string()
                .contains("restoring terminal echo settings")
        );
        assert_eq!(tcgetattr(&terminal).unwrap(), original);
    }

    // 실제 48열 PTY의 winsize를 읽은 confirmation 경로가 connect 핵심 정보와 exact profile을
    // 모두 보존하면서 모든 preview 물리 줄을 48셀 안에서 직접 감싸는지 검증합니다.
    #[test]
    fn confirmation_uses_the_controlling_tty_width_instead_of_shell_wrapping() {
        let pty = openpty(None, None).unwrap();
        tcsetwinsize(
            &pty.slave,
            Winsize {
                ws_row: 24,
                ws_col: 48,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        let complete = CompleteModelBinding::from_durable_json(
            r#"{"provider":"vendor","account":"team","model":"alpha","connector":"openai-responses","base_url":"https://long-provider.example.test/compatible-mode/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":4096,"max_output_tokens":128,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}"#,
        )
        .unwrap();
        let preview = Confirmation::Connect(Box::new(
            ConnectPreview::new(
                "vendor:team:alpha".to_owned(),
                "vendor:team".to_owned(),
                "unset  →  vendor:team:alpha".to_owned(),
                super::super::presentation::ManagedConnectionChange::Create,
                yo_core::CredentialMutationAction::Add,
                true,
                vec![BindingDetails::from(&complete)],
            )
            .with_verbose(true),
        ));
        let child = thread::spawn(move || {
            let mut input = TtyConnectionInput {
                terminal: Some(File::from(pty.slave)),
                style: PresentationStyle::Plain,
            };
            <TtyConnectionInput as ExternalConnectInput>::confirm(&mut input, &preview).unwrap()
        });

        fcntl(&pty.master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
        let mut master = File::from(pty.master);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut output = Vec::new();
        while !output
            .windows(b"Apply this connection plan? [y/N] ".len())
            .any(|window| window == b"Apply this connection plan? [y/N] ")
        {
            let mut buffer = [0_u8; 1024];
            match master.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => output.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "confirmation prompt did not appear"
                    );
                    thread::yield_now();
                },
                Err(error) => panic!("reading confirmation PTY failed: {error}"),
            }
        }
        write(&master, b"n\n").unwrap();
        assert!(!child.join().unwrap());

        let output = String::from_utf8(output).unwrap().replace('\r', "");
        let preview_output = output
            .split("Apply this connection plan? [y/N]")
            .next()
            .unwrap();
        assert!(preview_output.contains("CONNECT"));
        assert!(preview_output.contains("vendor:team:alpha"));
        assert!(preview_output.contains("semantic-terminal/v1"));
        for line in preview_output.lines() {
            assert!(
                cell_width(line).unwrap() <= 48,
                "48-cell PTY received overwide preview line {line:?}"
            );
        }
    }
}
