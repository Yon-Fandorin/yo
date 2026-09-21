//! 실행 중인 터미널 세대가 끝난 뒤 외부 편집기 프로세스를 소유한다.
//!
//! TUI가 터미널 모드를 복원한 뒤에만 호출한다. 편집기 프로세스 그룹에
//! 제어 터미널의 전경을 넘기고 초안은 새로 만든 비공개 Markdown 파일에 둔다.

use std::{
    env,
    error::Error,
    fmt, fs,
    fs::{DirBuilder, File, OpenOptions},
    io::{self, Read, Write},
    mem,
    os::{
        fd::AsFd,
        unix::{
            fs::{DirBuilderExt, OpenOptionsExt},
            process::{CommandExt, ExitStatusExt},
        },
    },
    path::{Path, PathBuf},
    process::{self, Child, Command, ExitStatus, Stdio},
    task::{Context, Poll, Waker},
    thread,
    time::{Duration, Instant},
};

use nix::{
    errno::Errno,
    sys::{
        signal::{
            SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal, killpg, pthread_sigmask,
            sigaction,
        },
        wait::{WaitPidFlag, WaitStatus, waitpid},
    },
    unistd::{Pid, getpgrp, tcgetpgrp, tcsetpgrp},
};

const MAX_DRAFT_BYTES: usize = 1024 * 1024;
const TEMP_DIRECTORY_ATTEMPTS: usize = 16;
const CHILD_SIGNALS: [Signal; 7] = [
    Signal::SIGHUP,
    Signal::SIGINT,
    Signal::SIGQUIT,
    Signal::SIGTERM,
    Signal::SIGTSTP,
    Signal::SIGTTIN,
    Signal::SIGTTOU,
];

/// 설정된 편집기를 실행하고 크기가 제한된 UTF-8 초안을 돌려준다.
///
/// TUI가 터미널 소유권을 반환한 뒤에만 호출한다. 호출자는 편집기를 기다리는
/// 동안 프로세스 호스트의 종료 정리 소유권을 유지한다. 셸은 실행하지 않고
/// 환경 변수의 명령을 argv로 해석해 `Command`에 직접 전달한다.
pub(crate) fn run(
    termination: &mut impl yo_tui::TerminationSource,
    draft: &str,
) -> Result<String, ExternalEditorError> {
    if draft.len() > MAX_DRAFT_BYTES {
        return Err(ExternalEditorError::message(format!(
            "current draft exceeds the {MAX_DRAFT_BYTES}-byte limit"
        )));
    }
    let editor = resolve_editor_command()?;
    let temporary = DraftFile::create(draft)?;
    let result = run_editor_process(termination, &editor, temporary.path());
    let result = result.and_then(|status| {
        if !status.success() {
            return Err(ExternalEditorError::message(format!(
                "editor exited with status {status}"
            )));
        }
        read_utf8_bounded(temporary.path())
    });
    let cleanup = temporary.cleanup();
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(ExternalEditorError::io(
            "cleaning up the editor draft",
            error,
        )),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup)) => Err(ExternalEditorError::message(format!(
            "{error}; additionally, cleaning up the editor draft failed: {cleanup}"
        ))),
    }
}

/// 비어 있지 않은 `VISUAL`을 우선하고, 없으면 `EDITOR`를 선택한다.
fn resolve_editor_command() -> Result<Vec<String>, ExternalEditorError> {
    let (name, value) = ["VISUAL", "EDITOR"]
        .into_iter()
        .filter_map(|name| {
            env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(|value| (name, value))
        })
        .next()
        .ok_or_else(|| ExternalEditorError::message("neither VISUAL nor EDITOR is set"))?;
    let value = value
        .to_str()
        .ok_or_else(|| ExternalEditorError::message(format!("{name} is not valid UTF-8")))?;
    let command = parse_argv(value).map_err(|error| {
        ExternalEditorError::message(format!("could not parse {name}: {error}"))
    })?;
    if command.is_empty() {
        return Err(ExternalEditorError::message(format!("{name} is empty")));
    }
    Ok(command)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Quote {
    Single,
    Double,
}

/// 셸 확장 없이 따옴표와 이스케이프를 이용한 인자 구분만 해석한다.
///
/// 따옴표 밖의 공백은 인자를 나눈다. `$`, 명령 치환, 리다이렉션, 글로브는
/// 셸을 거치지 않으므로 인자 안에 쓰인 그대로 남는다.
fn parse_argv(value: &str) -> Result<Vec<String>, ParseError> {
    let mut arguments = Vec::new();
    let mut argument = String::new();
    let mut quote = None;
    let mut started = false;
    let mut escaped = false;
    let mut characters = value.chars().peekable();

    while let Some(character) = characters.next() {
        if escaped {
            argument.push(character);
            started = true;
            escaped = false;
            continue;
        }
        match quote {
            Some(Quote::Single) => {
                if character == '\'' {
                    quote = None;
                } else {
                    argument.push(character);
                }
            },
            Some(Quote::Double) => match character {
                '"' => quote = None,
                '\\' => match characters.peek().copied() {
                    Some(next @ ('"' | '\\' | '$' | '`')) => {
                        argument.push(next);
                        characters.next();
                    },
                    Some('\n') => {
                        characters.next();
                    },
                    Some(_) | None => argument.push('\\'),
                },
                _ => argument.push(character),
            },
            None => match character {
                '\\' => {
                    escaped = true;
                    started = true;
                },
                '\'' => {
                    quote = Some(Quote::Single);
                    started = true;
                },
                '"' => {
                    quote = Some(Quote::Double);
                    started = true;
                },
                character if character.is_whitespace() => {
                    if started {
                        arguments.push(mem::take(&mut argument));
                        started = false;
                    }
                },
                _ => {
                    argument.push(character);
                    started = true;
                },
            },
        }
    }

    if escaped {
        return Err(ParseError::TrailingEscape);
    }
    if quote.is_some() {
        return Err(ParseError::UnterminatedQuote);
    }
    if started {
        arguments.push(argument);
    }
    Ok(arguments)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParseError {
    TrailingEscape,
    UnterminatedQuote,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TrailingEscape => "trailing backslash",
            Self::UnterminatedQuote => "unterminated quote",
        })
    }
}

struct DraftFile {
    directory: PathBuf,
    path: PathBuf,
}

impl DraftFile {
    fn create(draft: &str) -> Result<Self, ExternalEditorError> {
        let temporary_root = env::temp_dir();
        for _ in 0..TEMP_DIRECTORY_ATTEMPTS {
            let mut random = [0_u8; 16];
            getrandom::fill(&mut random).map_err(|error| {
                ExternalEditorError::message(format!("creating editor draft: {error}"))
            })?;
            let directory =
                temporary_root.join(format!("yo-editor-{}-{}", process::id(), hex(&random)));
            match DirBuilder::new().mode(0o700).create(&directory) {
                Ok(()) => {
                    let path = directory.join("draft.md");
                    let mut file = match OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(&path)
                    {
                        Ok(file) => file,
                        Err(error) => {
                            let _ = fs::remove_dir(&directory);
                            return Err(ExternalEditorError::io(
                                "creating the editor draft",
                                error,
                            ));
                        },
                    };
                    if let Err(error) = file.write_all(draft.as_bytes()) {
                        let _ = fs::remove_file(&path);
                        let _ = fs::remove_dir(&directory);
                        return Err(ExternalEditorError::io("writing the editor draft", error));
                    }
                    return Ok(Self { directory, path });
                },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(ExternalEditorError::io(
                        "creating the editor draft directory",
                        error,
                    ));
                },
            }
        }
        Err(ExternalEditorError::message(
            "could not allocate a private editor draft name",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn cleanup(self) -> io::Result<()> {
        remove_owned_path(&self.directory)
    }
}

impl Drop for DraftFile {
    fn drop(&mut self) {
        let _ = remove_owned_path(&self.directory);
    }
}

/// 심볼릭 링크를 따라가지 않고 비공개 편집기 디렉터리를 제거한다.
/// `remove_dir_all`은 링크를 항목으로 제거하므로 편집기가 `draft.md`를
/// 디렉터리로 바꾼 경우에도 정리할 수 있다.
fn remove_owned_path(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn read_utf8_bounded(path: &Path) -> Result<String, ExternalEditorError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| ExternalEditorError::io("reading the editor draft", error))?;
    if !metadata.file_type().is_file() {
        return Err(ExternalEditorError::message(
            "editor draft was replaced with a non-file",
        ));
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| ExternalEditorError::io("reading the editor draft", error))?;
    let opened = file
        .metadata()
        .map_err(|error| ExternalEditorError::io("reading the editor draft", error))?;
    if !opened.file_type().is_file() {
        return Err(ExternalEditorError::message(
            "editor draft was replaced with a non-file",
        ));
    }
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| ExternalEditorError::io("reading the editor draft", error))?;
        if count == 0 {
            break;
        }
        if bytes.len().saturating_add(count) > MAX_DRAFT_BYTES {
            return Err(ExternalEditorError::message(format!(
                "editor draft exceeds the {MAX_DRAFT_BYTES}-byte limit"
            )));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8(bytes)
        .map_err(|_| ExternalEditorError::message("editor draft is not valid UTF-8"))
}

fn run_editor_process(
    termination: &mut impl yo_tui::TerminationSource,
    command: &[String],
    draft: &Path,
) -> Result<ExitStatus, ExternalEditorError> {
    let mut terminal = ForegroundTerminal::open()?;
    let mut child = build_command(command, draft)
        .spawn()
        .map_err(|error| ExternalEditorError::io("starting the editor", error))?;
    let child_group = Pid::from_raw(
        i32::try_from(child.id())
            .map_err(|_| ExternalEditorError::message("editor process ID is out of range"))?,
    );
    if let Err(error) = terminal.give_to(child_group) {
        terminate_child(&mut child, child_group);
        let _ = terminal.restore();
        return Err(ExternalEditorError::io(
            "giving the terminal to the editor",
            error,
        ));
    }
    // spawn과 tcsetpgrp 사이에 자식이 곧바로 TTY를 읽으면 SIGTTIN으로 멈출 수 있다.
    // 전경 소유권을 넘긴 직후 시작 단계의 중지를 깨우고 이후 상태를 감시한다.
    let _ = killpg(child_group, Signal::SIGCONT);
    let status = wait_for_editor(termination, &mut child, child_group, true);
    let restore = terminal.restore();
    match (status, restore) {
        (Ok(status), Ok(())) => Ok(status),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(ExternalEditorError::io(
            "restoring the terminal foreground group",
            error,
        )),
        (Err(error), Err(restore)) => Err(ExternalEditorError::message(format!(
            "{error}; additionally, restoring the terminal foreground group failed: {restore}"
        ))),
    }
}

fn wait_for_editor(
    termination: &mut impl yo_tui::TerminationSource,
    child: &mut Child,
    process_group: Pid,
    mut recover_startup_sigttin: bool,
) -> Result<ExitStatus, ExternalEditorError> {
    loop {
        if termination_requested(termination) {
            terminate_child(child, process_group);
            return Err(ExternalEditorError::message(
                "editor interrupted by process termination; your draft was preserved",
            ));
        }

        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {},
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                terminate_child(child, process_group);
                return Err(ExternalEditorError::io("waiting for the editor", error));
            },
        }

        match waitpid(
            process_group,
            Some(WaitPidFlag::WNOHANG | WaitPidFlag::WUNTRACED),
        ) {
            Ok(WaitStatus::Stopped(_, Signal::SIGTTIN)) if recover_startup_sigttin => {
                recover_startup_sigttin = false;
                let _ = killpg(process_group, Signal::SIGCONT);
            },
            Ok(WaitStatus::Stopped(_, signal)) => {
                // Ctrl+Z는 전경 편집기 그룹을 멈춘다. 입력을 받을 수 없는 자식을
                // 계속 기다리지 않고 TTY를 회수해 이 편집 세대를 끝낸다.
                terminate_child(child, process_group);
                return Err(ExternalEditorError::message(format!(
                    "editor was suspended by {signal:?}; your draft was preserved"
                )));
            },
            Ok(WaitStatus::Exited(_, code)) => {
                return Ok(ExitStatus::from_raw(code << 8));
            },
            Ok(WaitStatus::Signaled(_, signal, dumped_core)) => {
                let raw = signal as i32 | if dumped_core { 0x80 } else { 0 };
                return Ok(ExitStatus::from_raw(raw));
            },
            Ok(WaitStatus::StillAlive | WaitStatus::Continued(_)) => {},
            #[cfg(target_os = "linux")]
            Ok(WaitStatus::PtraceEvent(_, _, _) | WaitStatus::PtraceSyscall(_)) => {},
            Err(error) if error == Errno::EINTR => continue,
            Err(error) => {
                terminate_child(child, process_group);
                return Err(ExternalEditorError::message(format!(
                    "waiting for the editor: {error}"
                )));
            },
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn termination_requested(termination: &mut impl yo_tui::TerminationSource) -> bool {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    matches!(
        termination.poll_termination(&mut context),
        Poll::Ready(yo_tui::TerminationEvent::Requested)
    )
}

#[allow(unsafe_code)]
fn build_command(command: &[String], draft: &Path) -> Command {
    let mut process = Command::new(&command[0]);
    process
        .args(&command[1..])
        .arg(draft)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .process_group(0);
    // 프로세스 그룹 설정은 CommandExt가 처리한다. pre-exec 훅은 편집기를
    // exec하기 전 호스트의 신호 상태만 지우며 할당이나 잠금을 수행하지 않는다.
    unsafe {
        process.pre_exec(reset_child_signal_state);
    }
    process
}

#[allow(unsafe_code)]
fn reset_child_signal_state() -> io::Result<()> {
    for signal in CHILD_SIGNALS {
        let action = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
        // SAFETY: fork된 자식에서 exec 전에 실행하며 빌린 핸들러 상태가 없는
        // 완전히 초기화된 신호 동작을 전달한다.
        unsafe { sigaction(signal, &action) }
            .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
    }
    pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&SigSet::empty()), None)
        .map_err(|error| io::Error::from_raw_os_error(error as i32))
}

fn terminate_child(child: &mut Child, process_group: Pid) {
    let _ = killpg(process_group, Signal::SIGCONT);
    let _ = killpg(process_group, Signal::SIGTERM);
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut reaped = false;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => {
                reaped = true;
                break;
            },
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
            Err(_) => {
                reaped = true;
                break;
            },
        }
    }
    let _ = killpg(process_group, Signal::SIGKILL);
    if !reaped {
        let _ = child.kill();
        let _ = child.wait();
    }
}

struct ForegroundTerminal {
    terminal: File,
    original_group: Pid,
    foreground: bool,
}

impl ForegroundTerminal {
    fn open() -> Result<Self, ExternalEditorError> {
        let terminal = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .map_err(|error| ExternalEditorError::io("opening the controlling terminal", error))?;
        let original_group = tcgetpgrp(&terminal).map_err(|error| {
            ExternalEditorError::io(
                "reading the terminal foreground group",
                io::Error::from_raw_os_error(error as i32),
            )
        })?;
        if original_group != getpgrp() {
            return Err(ExternalEditorError::message(
                "the current Yo process is not the terminal foreground group",
            ));
        }
        Ok(Self {
            terminal,
            original_group,
            foreground: false,
        })
    }

    fn give_to(&mut self, process_group: Pid) -> io::Result<()> {
        set_foreground(&self.terminal, process_group)?;
        self.foreground = true;
        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        if !self.foreground {
            return Ok(());
        }
        set_foreground(&self.terminal, self.original_group)?;
        self.foreground = false;
        Ok(())
    }
}

impl Drop for ForegroundTerminal {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn set_foreground(terminal: &impl AsFd, process_group: Pid) -> io::Result<()> {
    let mut signal_set = SigSet::empty();
    signal_set.add(Signal::SIGTTOU);
    let mut previous = SigSet::empty();
    pthread_sigmask(
        SigmaskHow::SIG_BLOCK,
        Some(&signal_set),
        Some(&mut previous),
    )
    .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
    let result = tcsetpgrp(terminal, process_group)
        .map_err(|error| io::Error::from_raw_os_error(error as i32));
    let restore = pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&previous), None)
        .map_err(|error| io::Error::from_raw_os_error(error as i32));
    match (result, restore) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(restore)) => Err(io::Error::other(format!(
            "{error}; restoring the signal mask failed: {restore}"
        ))),
    }
}

#[derive(Debug)]
pub(crate) struct ExternalEditorError {
    detail: String,
}

impl ExternalEditorError {
    fn message(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }

    fn io(context: &str, error: io::Error) -> Self {
        Self::message(format!("{context}: {error}"))
    }
}

impl fmt::Display for ExternalEditorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl Error for ExternalEditorError {}

#[cfg(test)]
mod tests;
