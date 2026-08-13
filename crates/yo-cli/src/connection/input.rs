use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
};

use nix::sys::termios::{self, LocalFlags, SetArg, Termios};
use yo_core::ApiCredential;

use crate::AppError;

const MAX_INPUT_BYTES: usize = 16 * 1024;

pub(super) trait ExternalConnectInput {
    fn confirm(&mut self, summary: &str) -> Result<bool, AppError>;
    fn read_credential(&mut self, account: &str) -> Result<ApiCredential, AppError>;
}

pub(super) trait ExternalDisconnectInput {
    fn select_target(&mut self, choices: &[String]) -> Result<String, AppError>;
    fn confirm(&mut self, summary: &str) -> Result<bool, AppError>;
}

pub(super) struct TtyConnectionInput {
    terminal: Option<File>,
}

impl TtyConnectionInput {
    pub(super) const fn new() -> Self {
        Self { terminal: None }
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
    fn confirm(&mut self, summary: &str) -> Result<bool, AppError> {
        let terminal = self.terminal()?;
        write!(terminal, "{summary}\nContinue? [y/N] ")
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

    fn confirm(&mut self, summary: &str) -> Result<bool, AppError> {
        <Self as ExternalConnectInput>::confirm(self, summary)
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

    use super::*;

    // 실제 PTY에서 API key 입력 동안 ECHO가 꺼져 master 출력에 secret이 나타나지 않고,
    // 성공 뒤에는 호출 전 termios 전체가 정확히 복구됩니다.
    #[test]
    fn credential_input_hides_secret_and_restores_exact_termios() {
        let pty = openpty(None, None).unwrap();
        let observed_slave = pty.slave.try_clone().unwrap();
        let original = tcgetattr(&observed_slave).unwrap();
        let child = thread::spawn(move || {
            let mut input = TtyConnectionInput {
                terminal: Some(File::from(pty.slave)),
            };
            input.read_credential("vendor:team").unwrap()
        });

        let deadline = Instant::now() + Duration::from_secs(2);
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
        write(&pty.master, b"sentinel-secret\n").unwrap();
        assert_eq!(child.join().unwrap().expose_secret(), "sentinel-secret");
        assert_eq!(tcgetattr(&observed_slave).unwrap(), original);

        fcntl(&pty.master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
        let mut master = File::from(pty.master);
        let mut output = Vec::new();
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
}
