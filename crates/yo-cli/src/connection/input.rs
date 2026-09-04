use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
};

use nix::sys::termios::{self, LocalFlags, SetArg, SpecialCharacterIndices, Termios, tcsetattr};
use rustix::termios::{QueueSelector, tcflush};
use yo_core::ApiCredential;

mod file;
mod picker;

pub(super) use file::AuthorizedCredentialFileInput;
pub(super) use picker::ModelPickerItem;

use super::presentation::{Confirmation, HiddenSecretPrompt, default_width};
use crate::{AppError, presentation::PresentationStyle};

const MAX_INPUT_BYTES: usize = 16 * 1024;

pub(super) trait ExternalConnectInput {
    fn confirm(&mut self, preview: &Confirmation) -> Result<bool, AppError>;
    fn read_credential(&mut self, account: &str) -> Result<ApiCredential, AppError>;
    fn select_model(&mut self, models: &[ModelPickerItem]) -> Result<Option<usize>, AppError> {
        let _ = models;
        Err(AppError::message(
            "model selection requires an interactive controlling terminal",
        ))
    }
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

    #[cfg(test)]
    pub(super) fn with_terminal(terminal: File) -> Self {
        Self {
            terminal: Some(terminal),
            style: PresentationStyle::Plain,
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
            return match tcflush(terminal, QueueSelector::IFlush) {
                Ok(()) => Err(AppError::message(format!(
                    "terminal input exceeds the {MAX_INPUT_BYTES}-byte limit"
                ))),
                Err(error) => Err(AppError::single(
                    "discarding oversized terminal input",
                    error,
                )),
            };
        }
        String::from_utf8(bytes)
            .map_err(|_| AppError::message("terminal input must be valid UTF-8"))
    }

    fn read_secret_with(
        &mut self,
        prompt: &str,
        validation_context: &'static str,
        read: impl FnOnce(&File) -> Result<String, AppError>,
    ) -> Result<ApiCredential, AppError> {
        let terminal = self.terminal()?;
        write!(terminal, "{prompt}")
            .and_then(|()| terminal.flush())
            .map_err(|error| AppError::single("writing the hidden-secret prompt", error))?;
        let original = termios::tcgetattr(&*terminal)
            .map_err(|error| AppError::single("reading terminal echo settings", error))?;
        let mut hidden = original.clone();
        hidden
            .local_flags
            .remove(LocalFlags::ECHO | LocalFlags::ICANON);
        hidden.control_chars[SpecialCharacterIndices::VMIN as usize] = 1;
        hidden.control_chars[SpecialCharacterIndices::VTIME as usize] = 0;
        tcsetattr(&*terminal, SetArg::TCSAFLUSH, &hidden)
            .map_err(|error| AppError::single("disabling terminal echo", error))?;
        let restore = EchoRestore {
            terminal,
            original: Some(original),
        };
        let value = read(terminal);
        let restore_result = restore.restore();
        writeln!(terminal)
            .and_then(|()| terminal.flush())
            .map_err(|error| AppError::single("finishing the hidden-secret prompt", error))?;
        restore_result?;
        ApiCredential::new(value?).map_err(|error| AppError::single(validation_context, error))
    }

    fn read_credential_with(
        &mut self,
        account: &str,
        read: impl FnOnce(&File) -> Result<String, AppError>,
    ) -> Result<ApiCredential, AppError> {
        self.read_secret_with(
            &format!("API key for {account}: "),
            "reading the API key",
            read,
        )
    }
}

pub(crate) fn read_hidden_secret(
    prompt: &HiddenSecretPrompt,
    validation_context: &'static str,
) -> Result<ApiCredential, AppError> {
    let mut input = TtyConnectionInput::new();
    let terminal_width = {
        let terminal = input.terminal()?;
        rustix::termios::tcgetwinsize(&*terminal)
            .ok()
            .and_then(|size| std::num::NonZeroU16::new(size.ws_col))
            .unwrap_or_else(default_width)
    };
    let rendered = prompt
        .render_styled(terminal_width, input.style)
        .map_err(|error| AppError::single("formatting the hidden-secret prompt", error))?;
    input.read_secret_with(&rendered, validation_context, TtyConnectionInput::read_line)
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
        tcflush(&*terminal, QueueSelector::IFlush).map_err(|error| {
            AppError::single(
                "discarding queued terminal input before confirmation",
                error,
            )
        })?;
        write!(terminal, "{rendered}\n\n{}", preview.prompt())
            .and_then(|()| terminal.flush())
            .map_err(|error| AppError::single("writing the connection confirmation", error))?;
        let answer = Self::read_line(terminal)?;
        Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "YES" | "Yes"))
    }

    fn read_credential(&mut self, account: &str) -> Result<ApiCredential, AppError> {
        self.read_credential_with(account, Self::read_line)
    }

    fn select_model(&mut self, models: &[ModelPickerItem]) -> Result<Option<usize>, AppError> {
        let style = self.style;
        picker::select_model(self.terminal()?, models, style)
    }
}

impl ExternalDisconnectInput for TtyConnectionInput {
    fn select_target(&mut self, choices: &[String]) -> Result<String, AppError> {
        let terminal = self.terminal()?;
        write!(
            terminal,
            "Select one stored target by entering its exact reference:\n  - {}\nTarget: ",
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
        self.restore_with(|terminal, original| tcsetattr(terminal, SetArg::TCSAFLUSH, original))
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
            let _ = tcsetattr(self.terminal, SetArg::TCSAFLUSH, original);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File},
        io::{Read, Write},
        path::PathBuf,
        sync::mpsc,
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use nix::{
        fcntl::{FcntlArg, OFlag, fcntl},
        pty::openpty,
        sys::termios::{LocalFlags, SetArg, SpecialCharacterIndices, tcgetattr, tcsetattr},
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

    fn immediate_input_terminal() -> (File, File, File, Termios) {
        let pty = openpty(None, None).unwrap();
        let observed = pty.slave.try_clone().unwrap();
        let terminal = File::from(pty.slave);
        let master = File::from(pty.master);
        let mut mode = tcgetattr(&terminal).unwrap();
        mode.local_flags
            .remove(LocalFlags::ICANON | LocalFlags::ECHO);
        mode.control_chars[SpecialCharacterIndices::VMIN as usize] = 1;
        mode.control_chars[SpecialCharacterIndices::VTIME as usize] = 0;
        tcsetattr(&terminal, SetArg::TCSANOW, &mode).unwrap();
        (terminal, File::from(observed), master, mode)
    }

    fn wait_for_credential_mode(terminal: &File, original: &Termios, deadline: Instant) {
        loop {
            let current = tcgetattr(terminal).unwrap();
            if !current.local_flags.contains(LocalFlags::ECHO)
                && !current.local_flags.contains(LocalFlags::ICANON)
            {
                assert_eq!(
                    current.local_flags.contains(LocalFlags::ISIG),
                    original.local_flags.contains(LocalFlags::ISIG)
                );
                assert_eq!(
                    current.control_chars[SpecialCharacterIndices::VMIN as usize],
                    1
                );
                assert_eq!(
                    current.control_chars[SpecialCharacterIndices::VTIME as usize],
                    0
                );
                return;
            }
            assert!(
                Instant::now() < deadline,
                "credential prompt did not enter guarded noncanonical mode"
            );
            thread::yield_now();
        }
    }

    fn read_credential_prompt(master: &mut File, deadline: Instant) {
        let expected = b"API key for vendor:team: ";
        let mut observed = vec![0_u8; expected.len()];
        let mut offset = 0_usize;
        while offset < observed.len() {
            match master.read(&mut observed[offset..]) {
                Ok(0) => panic!("credential prompt closed its PTY before completing"),
                Ok(count) => offset += count,
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
        assert_eq!(observed, expected);
    }

    fn write_all_until(file: &mut File, mut bytes: &[u8], deadline: Instant) {
        while !bytes.is_empty() {
            match file.write(bytes) {
                Ok(0) => panic!("PTY stopped accepting credential input"),
                Ok(count) => bytes = &bytes[count..],
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "credential PTY write deadline expired"
                    );
                    thread::yield_now();
                },
                Err(error) => panic!("writing credential PTY failed: {error}"),
            }
        }
    }

    // 4,095·4,096·16,384-byte credential은 guarded noncanonical mode에서 canonical line
    // capacity와 무관하게 exact bytes로 도착하고, ISIG와 호출 전 termios는 그대로 보존됩니다.
    #[test]
    fn credential_input_preserves_all_in_range_bytes_in_noncanonical_mode() {
        for length in [4_095, 4_096, MAX_INPUT_BYTES] {
            let pty = openpty(None, None).unwrap();
            let observed = File::from(pty.slave.try_clone().unwrap());
            let original = tcgetattr(&observed).unwrap();
            let (result_tx, result_rx) = mpsc::channel();
            let worker = thread::spawn(move || {
                let mut input = TtyConnectionInput::with_terminal(File::from(pty.slave));
                let result = input
                    .read_credential("vendor:team")
                    .map(|credential| credential.expose_secret().to_owned())
                    .map_err(|error| error.to_string());
                result_tx.send(result).unwrap();
            });
            let mut master = File::from(pty.master);
            fcntl(&master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            read_credential_prompt(&mut master, deadline);
            wait_for_credential_mode(&observed, &original, deadline);
            let mut secret = vec![b's'; length];
            secret.push(b'\n');
            write_all_until(&mut master, &secret, deadline);

            let captured = result_rx
                .recv_timeout(Duration::from_secs(3))
                .expect("credential capture did not finish")
                .expect("in-range credential was rejected");
            worker.join().unwrap();

            assert_eq!(captured.as_bytes(), &secret[..length]);
            assert_eq!(tcgetattr(&observed).unwrap(), original);
        }
    }

    // invalid UTF-8와 주입한 read failure 모두 secret admission 전에 실패하지만 guarded
    // no-echo/noncanonical capture는 두 경로에서 exact original termios를 먼저 복구합니다.
    #[test]
    fn credential_input_restores_after_invalid_utf8_and_read_failure() {
        let pty = openpty(None, None).unwrap();
        let observed = File::from(pty.slave.try_clone().unwrap());
        let original = tcgetattr(&observed).unwrap();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut input = TtyConnectionInput::with_terminal(File::from(pty.slave));
            let result = input
                .read_credential("vendor:team")
                .map(|_| ())
                .map_err(|error| error.to_string());
            result_tx.send(result).unwrap();
        });
        let mut master = File::from(pty.master);
        fcntl(&master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        read_credential_prompt(&mut master, deadline);
        wait_for_credential_mode(&observed, &original, deadline);
        write_all_until(&mut master, &[0xff, b'\n'], deadline);
        let error = result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("invalid UTF-8 capture did not finish")
            .expect_err("invalid UTF-8 unexpectedly produced a credential");
        worker.join().unwrap();
        assert!(error.contains("valid UTF-8"));
        assert_eq!(tcgetattr(&observed).unwrap(), original);

        let pty = openpty(None, None).unwrap();
        let observed = File::from(pty.slave.try_clone().unwrap());
        let original = tcgetattr(&observed).unwrap();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut input = TtyConnectionInput::with_terminal(File::from(pty.slave));
            let result = input.read_credential_with("vendor:team", |terminal| {
                let current = tcgetattr(terminal).unwrap();
                assert!(!current.local_flags.contains(LocalFlags::ECHO));
                assert!(!current.local_flags.contains(LocalFlags::ICANON));
                Err(AppError::message("injected credential read failure"))
            });
            result_tx.send(result).unwrap();
        });
        let mut master = File::from(pty.master);
        fcntl(&master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
        read_credential_prompt(&mut master, Instant::now() + Duration::from_secs(2));
        let error = result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("injected credential read failure did not finish")
            .unwrap_err();
        worker.join().unwrap();
        assert!(
            error
                .to_string()
                .contains("injected credential read failure")
        );
        assert_eq!(tcgetattr(&observed).unwrap(), original);
    }

    // 한도 직전과 exact 한도 입력은 그대로 읽고, limit+1 뒤 이미 queue에 들어온 tail은
    // TCIFLUSH로 폐기되어 다음 prompt나 shell reader에 sentinel을 넘기지 않습니다.
    #[test]
    fn terminal_line_overflow_flushes_the_complete_pending_input_queue() {
        for length in [MAX_INPUT_BYTES - 1, MAX_INPUT_BYTES] {
            let (terminal, _observed, mut master, _mode) = immediate_input_terminal();
            let reader = thread::spawn(move || TtyConnectionInput::read_line(&terminal).unwrap());
            let mut input = vec![b'a'; length];
            input.push(b'\n');
            master.write_all(&input).unwrap();
            assert_eq!(reader.join().unwrap().len(), length);
        }

        let (terminal, mut observed, mut master, _mode) = immediate_input_terminal();
        let reader = thread::spawn(move || TtyConnectionInput::read_line(&terminal).unwrap_err());
        let mut input = vec![b'a'; MAX_INPUT_BYTES + 1];
        input.extend_from_slice(b"shell-sentinel\n");
        master.write_all(&input).unwrap();
        let error = reader.join().unwrap();

        assert!(error.to_string().contains("terminal input exceeds"));
        fcntl(&observed, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
        let mut residue = [0_u8; 64];
        let residue = observed.read(&mut residue).unwrap_err();
        assert_eq!(residue.kind(), std::io::ErrorKind::WouldBlock);
    }

    // 입력 한도를 넘긴 descriptor가 TTY가 아니어서 TCIFLUSH 자체가 실패하면 size error와
    // 다른 cleanup diagnostic을 반환해 queue 격리를 보장하지 못한 사실을 숨기지 않습니다.
    #[test]
    fn terminal_line_overflow_reports_a_distinct_flush_failure() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = PathBuf::from(format!(
            "/tmp/yo-terminal-overflow-{}-{nonce}",
            std::process::id()
        ));
        fs::write(&path, vec![b'a'; MAX_INPUT_BYTES + 1]).unwrap();
        let file = File::open(&path).unwrap();

        let error = TtyConnectionInput::read_line(&file).unwrap_err();
        let _ = fs::remove_file(path);

        assert!(
            error
                .to_string()
                .contains("discarding oversized terminal input")
        );
    }

    // Credential 입력 overflow도 size/flush 오류를 반환하기 전에 no-echo guard가 original
    // termios를 복구하므로 실패한 oversized secret 뒤에 숨김 mode가 남지 않습니다.
    #[test]
    fn oversized_credential_restores_exact_terminal_mode_before_returning() {
        let (terminal, observed, mut master, mut original) = immediate_input_terminal();
        original.local_flags.insert(LocalFlags::ECHO);
        tcsetattr(&terminal, SetArg::TCSANOW, &original).unwrap();
        let child = thread::spawn(move || {
            let mut input = TtyConnectionInput {
                terminal: Some(terminal),
                style: PresentationStyle::Plain,
            };
            input.read_credential("vendor:team").unwrap_err()
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        fcntl(&master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
        read_credential_prompt(&mut master, deadline);
        while tcgetattr(&observed)
            .unwrap()
            .local_flags
            .contains(LocalFlags::ECHO)
        {
            assert!(Instant::now() < deadline, "credential echo stayed enabled");
            thread::yield_now();
        }
        let mut secret = vec![b's'; MAX_INPUT_BYTES + 1];
        secret.push(b'\n');
        write_all_until(&mut master, &secret, deadline);

        let error = child.join().unwrap();

        assert!(error.to_string().contains("terminal input exceeds"));
        assert_eq!(tcgetattr(&observed).unwrap(), original);
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
        tcsetattr(&terminal, SetArg::TCSAFLUSH, &hidden).unwrap();

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
            r#"{"provider":"vendor","account":"team","model":"alpha","connector":"openai-responses","base_url":"https://long-provider.example.test/compatible-mode/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":4096,"max_output_tokens":128,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
        )
        .unwrap();
        let preview = Confirmation::Connect(Box::new(
            ConnectPreview::new(
                "vendor:team:alpha".to_owned(),
                "vendor:team".to_owned(),
                "unset  →  vendor:team:alpha".to_owned(),
                super::super::presentation::StoredConnectionChange::Create,
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
        assert!(preview_output.contains("semantic-only/v1"));
        for line in preview_output.lines() {
            assert!(
                cell_width(line).unwrap() <= 48,
                "48-cell PTY received overwide preview line {line:?}"
            );
        }
    }

    // preview를 모두 만든 뒤 TCIFLUSH가 실패하면 prompt bytes를 한 바이트도 게시하지 않고
    // queued-input 격리 실패를 별도 diagnostic으로 반환해 확인 경계를 흐리지 않습니다.
    #[test]
    fn confirmation_flush_failure_precedes_preview_publication() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = PathBuf::from(format!(
            "/tmp/yo-confirmation-flush-{}-{nonce}",
            std::process::id()
        ));
        let terminal = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let complete = CompleteModelBinding::from_durable_json(
            r#"{"provider":"vendor","account":"team","model":"alpha","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":4096,"max_output_tokens":128,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
        )
        .unwrap();
        let preview = Confirmation::Connect(Box::new(ConnectPreview::new(
            "vendor:team:alpha".to_owned(),
            "vendor:team".to_owned(),
            "unset  →  vendor:team:alpha".to_owned(),
            super::super::presentation::StoredConnectionChange::Create,
            yo_core::CredentialMutationAction::Add,
            true,
            vec![BindingDetails::from(&complete)],
        )));
        let mut input = TtyConnectionInput::with_terminal(terminal);

        let error = <TtyConnectionInput as ExternalConnectInput>::confirm(&mut input, &preview)
            .unwrap_err();
        drop(input);
        let bytes = fs::read(&path).unwrap();
        let _ = fs::remove_file(path);

        assert!(
            error
                .to_string()
                .contains("discarding queued terminal input before confirmation")
        );
        assert!(bytes.is_empty());
    }
}
