//! Session-owned host presentation; provider actions remain on the original connection.
use std::{
    collections::BTreeMap,
    io::{self, Read},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Condvar, Mutex},
    task::{Context, Poll, Waker},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use nix::{
    fcntl::{FcntlArg, OFlag, fcntl},
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use yo_core::LocalWorkspaceReferenceProvider;
use yo_tui::{
    AgentAction, AgentConnection, AgentPoll, DispatchOutcome, LinkResolver, PendingDispatch,
    TuiStatusLine, surface::Hyperlink,
};

#[derive(Default)]
struct State {
    stop: bool,
    pending: Option<TuiStatusLine>,
    links: Option<Option<LinkResolver>>,
    waker: Option<Waker>,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

pub(super) struct Presentation {
    shared: Shared,
    worker: Option<JoinHandle<()>>,
    host_turn: bool,
    closed: bool,
}

impl Presentation {
    pub(super) fn start(root: PathBuf) -> Self {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("yo-host-presentation".into())
            .spawn(move || worker(&root, &worker_shared));
        if worker.is_err() {
            publish(&shared, status("Git status unavailable"));
        }
        Self {
            shared,
            worker: worker.ok(),
            host_turn: true,
            closed: false,
        }
    }

    pub(super) fn connection<'a, A: AgentConnection>(
        &'a mut self,
        agent: &'a mut A,
    ) -> Connection<'a, A> {
        Connection {
            presentation: self,
            agent,
        }
    }

    fn take(&self) -> Option<TuiStatusLine> {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending
            .take()
    }
}

impl Drop for Presentation {
    fn drop(&mut self) {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stop = true;
        self.shared.1.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) struct Connection<'a, A: AgentConnection> {
    presentation: &'a mut Presentation,
    agent: &'a mut A,
}

impl<A: AgentConnection> AgentConnection for Connection<'_, A> {
    type Error = A::Error;
    fn dispatch(&mut self, action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        self.agent.dispatch(action)
    }
    fn retry(&mut self, pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        self.agent.retry(pending)
    }
    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        if self.presentation.closed {
            return Ok(AgentPoll::Closed);
        }
        if self.presentation.host_turn {
            let update = self
                .presentation
                .take()
                .map(AgentPoll::StatusLine)
                .or_else(|| {
                    self.presentation
                        .shared
                        .0
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .links
                        .take()
                        .map(AgentPoll::Links)
                });
            if let Some(update) = update {
                self.presentation.host_turn = false;
                return Ok(update);
            }
        }
        self.presentation.host_turn = true;
        let result = self.agent.poll();
        if matches!(result, Ok(AgentPoll::Closed)) {
            self.presentation.closed = true;
            self.presentation
                .shared
                .0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .stop = true;
            self.presentation.shared.1.notify_all();
        }
        result
    }
    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        let host_ready = {
            let mut state = self
                .presentation
                .shared
                .0
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.waker = Some(context.waker().clone());
            state.pending.is_some() || state.links.is_some()
        };
        // Always register the original source, even while host updates are available.
        let agent_ready = self.agent.poll_ready(context);
        if host_ready || agent_ready.is_ready() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

fn status(value: &str) -> TuiStatusLine {
    TuiStatusLine::new([("git", value)]).unwrap_or_default()
}

fn publish(shared: &Shared, status: TuiStatusLine) {
    let waker = {
        let mut state = shared.0.lock().unwrap_or_else(|error| error.into_inner());
        state.pending = Some(status);
        state.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

fn stopped(shared: &Shared) -> bool {
    shared
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .stop
}

fn worker(root: &Path, shared: &Shared) {
    let mut previous = None;
    let mut previous_links = None;
    let canonical = std::fs::canonicalize(root).ok();
    while !stopped(shared) {
        let current = status(&branch(root, shared));
        if previous.as_ref() != Some(&current) {
            publish(shared, current.clone());
            previous = Some(current);
        }
        let links = canonical
            .as_ref()
            .and_then(|canonical| {
                if std::fs::canonicalize(root).ok().as_ref() != Some(canonical) {
                    return None;
                }
                let result = run_with_limit(
                    LocalWorkspaceReferenceProvider::output_link_command(canonical),
                    shared,
                    Duration::from_secs(2),
                    4 * 1024 * 1024,
                );
                let bytes = result
                    .ok()
                    .filter(|(exit, _)| exit.success())
                    .map(|(_, bytes)| bytes);
                LocalWorkspaceReferenceProvider::output_links(canonical, bytes.as_deref(), || {
                    stopped(shared)
                })
                .ok()
            })
            .unwrap_or_default();
        if previous_links.as_ref() != Some(&links) {
            let update = resolver(&links);
            let waker = {
                let mut state = shared.0.lock().unwrap_or_else(|error| error.into_inner());
                state.links = Some(update);
                state.waker.take()
            };
            if let Some(waker) = waker {
                waker.wake();
            }
            previous_links = Some(links);
        }
        let state = shared.0.lock().unwrap_or_else(|error| error.into_inner());
        let _ = shared
            .1
            .wait_timeout_while(state, Duration::from_secs(5), |state| !state.stop);
    }
}

fn resolver(paths: &BTreeMap<String, PathBuf>) -> Option<LinkResolver> {
    let mut links = BTreeMap::new();
    for (relative, path) in paths {
        if let Some(link) = Hyperlink::from_file_path(path) {
            links.insert(relative.clone(), link.clone());
            if let Some(absolute) = path.to_str() {
                links.insert(absolute.to_owned(), link);
            }
        }
    }
    (!links.is_empty())
        .then(|| LinkResolver::new(move |destination| resolve_link(&links, destination)))
}

fn resolve_link(links: &BTreeMap<String, Hyperlink>, destination: &str) -> Option<Hyperlink> {
    // Decode URL path escapes exactly once. '+' remains a literal path character;
    // encoded paths can only select an already validated inventory entry.
    let mut decoded = Vec::with_capacity(destination.len());
    let mut bytes = destination.as_bytes().iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = char::from(bytes.next()?).to_digit(16)?;
            let low = char::from(bytes.next()?).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    let path = std::str::from_utf8(&decoded).ok()?;
    links.get(path.strip_prefix("./").unwrap_or(path)).cloned()
}

fn branch(root: &Path, shared: &Shared) -> String {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"]);
    // Ambient Git overrides must not select a different repository or inject config.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command.env("LC_ALL", "C").env("GIT_TERMINAL_PROMPT", "0");
    match run(command, shared, Duration::from_secs(2)) {
        Ok((exit, bytes)) if exit.success() => {
            match std::str::from_utf8(&bytes)
                .ok()
                .map(str::trim)
                .filter(|value| {
                    !value.is_empty() && value.len() <= 200 && !value.chars().any(char::is_control)
                }) {
                Some(value) => format!("Git · {value}"),
                None => "Git status unavailable".into(),
            }
        },
        Ok((exit, _)) if exit.code() == Some(1) => "Git · detached HEAD".into(),
        _ => "Git status unavailable".into(),
    }
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Ok(pid) = i32::try_from(self.0.id()) {
            let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run(command: Command, shared: &Shared, timeout: Duration) -> io::Result<(ExitStatus, Vec<u8>)> {
    run_with_limit(command, shared, timeout, 1024)
}

fn run_with_limit(
    mut command: Command,
    shared: &Shared,
    timeout: Duration,
    limit: usize,
) -> io::Result<(ExitStatus, Vec<u8>)> {
    if stopped(shared) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let mut child = OwnedChild(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?,
    );
    let mut output = child.0.stdout.take().expect("piped status output");
    let flags = OFlag::from_bits_truncate(fcntl(&output, FcntlArg::F_GETFL)?);
    fcntl(&output, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    loop {
        if stopped(shared) || Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let mut buffer = [0; 256];
        loop {
            match output.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if bytes.len() + count > limit {
                        return Err(io::ErrorKind::InvalidData.into());
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                },
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        if let Some(exit) = child.0.try_wait()? {
            // Drain bytes that arrived between the preceding read and process exit.
            output.take((limit + 1) as u64).read_to_end(&mut bytes)?;
            if bytes.len() > limit {
                return Err(io::ErrorKind::InvalidData.into());
            }
            return Ok((exit, bytes));
        }
        let state = shared.0.lock().unwrap_or_else(|error| error.into_inner());
        let _ = shared
            .1
            .wait_timeout_while(state, Duration::from_millis(10), |state| !state.stop);
    }
}

#[cfg(test)]
mod tests;
