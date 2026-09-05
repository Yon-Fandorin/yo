mod file;

pub(crate) use file::AuthorizedCredentialFileInput;
use yo_core::ApiCredential;

pub(crate) use super::picker::ModelPickerItem;
use crate::{
    AppError,
    connection::{input::TtyConnectionInput, presentation::ConfirmationView},
};

pub(crate) trait ExternalConnectInput {
    fn confirm(&mut self, preview: &dyn ConfirmationView) -> Result<bool, AppError>;
    fn read_credential(&mut self, account: &str) -> Result<ApiCredential, AppError>;
    fn select_model(&mut self, models: &[ModelPickerItem]) -> Result<Option<usize>, AppError> {
        let _ = models;
        Err(AppError::message(
            "model selection requires an interactive controlling terminal",
        ))
    }
}

impl ExternalConnectInput for TtyConnectionInput {
    fn confirm(&mut self, preview: &dyn ConfirmationView) -> Result<bool, AppError> {
        TtyConnectionInput::confirm(self, preview)
    }

    fn read_credential(&mut self, account: &str) -> Result<ApiCredential, AppError> {
        TtyConnectionInput::read_credential(self, account)
    }

    fn select_model(&mut self, models: &[ModelPickerItem]) -> Result<Option<usize>, AppError> {
        let style = self.style();
        let terminal = self.terminal()?;
        picker::select_model(terminal, models, style)
    }
}

use super::picker;
#[cfg(test)]
mod confirmation_tests {
    use std::{
        fs::{self, File, OpenOptions},
        io::Read,
        path::PathBuf,
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use nix::{
        fcntl::{FcntlArg, OFlag, fcntl},
        pty::openpty,
        unistd::write,
    };
    use rustix::termios::{Winsize, tcsetwinsize};
    use yo_core::CompleteModelBinding;
    use yo_tui::surface::cell_width;

    use super::super::presentation::{Confirmation, ConnectPreview, StoredConnectionChange};
    use crate::connection::{input::TtyConnectionInput, presentation::BindingDetails};
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
                StoredConnectionChange::Create,
                yo_core::CredentialMutationAction::Add,
                true,
                vec![BindingDetails::from(&complete)],
            )
            .with_verbose(true),
        ));
        let child = thread::spawn(move || {
            let mut input = TtyConnectionInput::with_terminal(File::from(pty.slave));
            input.confirm(&preview).unwrap()
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
            StoredConnectionChange::Create,
            yo_core::CredentialMutationAction::Add,
            true,
            vec![BindingDetails::from(&complete)],
        )));
        let mut input = TtyConnectionInput::with_terminal(terminal);

        let error = input.confirm(&preview).unwrap_err();
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
