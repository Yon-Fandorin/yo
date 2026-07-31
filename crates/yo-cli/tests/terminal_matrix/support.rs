use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::Command,
};

use nix::{
    sys::termios::{LocalFlags, Termios, tcgetattr},
    unistd::Pid,
};

pub(crate) fn repository_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate is under <repository>/crates/yo-cli")
        .canonicalize()
        .expect("canonicalize repository")
}

pub(crate) fn read_termios(tty_path: &Path) -> Option<Termios> {
    let tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open(tty_path)
        .ok()?;
    tcgetattr(&tty).ok()
}

pub(crate) fn has_noncanonical_no_echo_input(tty_path: &Path) -> bool {
    read_termios(tty_path).is_some_and(|termios| {
        !termios
            .local_flags
            .intersects(LocalFlags::ICANON | LocalFlags::ECHO)
    })
}

pub(crate) fn only_child(parent: Pid) -> Option<Pid> {
    let output = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let rows = String::from_utf8(output.stdout).ok()?;
    let mut children = rows.lines().filter_map(|line| {
        let mut fields = line.split_whitespace();
        let pid = fields.next()?.parse::<i32>().ok()?;
        let ppid = fields.next()?.parse::<i32>().ok()?;
        (ppid == parent.as_raw()).then_some(Pid::from_raw(pid))
    });
    let child = children.next()?;
    children.next().is_none().then_some(child)
}

pub(crate) fn process_is_stopped(pid: Pid) -> bool {
    Command::new("/bin/ps")
        .args(["-o", "stat=", "-p", &pid.as_raw().to_string()])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|state| state.trim_start().starts_with('T'))
}

pub(crate) fn process_exists(pid: Pid) -> bool {
    nix::sys::signal::kill(pid, None).is_ok()
}

#[cfg(target_os = "linux")]
pub(crate) fn position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|candidate| candidate == needle)
}

#[cfg(target_os = "linux")]
pub(crate) fn last_position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|candidate| candidate == needle)
}

#[cfg(target_os = "linux")]
pub(crate) fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|candidate| *candidate == needle)
        .count()
}

#[cfg(target_os = "linux")]
pub(crate) fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    position(haystack, needle).is_some()
}

pub(crate) fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}

pub(crate) fn require_command(command: &str, arguments: &[&str]) {
    let output = Command::new(command)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("required command `{command}` is unavailable: {error}"));
    assert!(
        output.status.success(),
        "required command `{command}` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
