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
    let children =
        std::fs::read_to_string(format!("/proc/{0}/task/{0}/children", parent.as_raw())).ok()?;
    let mut children = children.split_whitespace();
    let child = children.next()?.parse::<i32>().ok()?;
    children.next().is_none().then(|| Pid::from_raw(child))
}

pub(crate) fn process_is_stopped(pid: Pid) -> bool {
    std::fs::read_to_string(format!("/proc/{}/stat", pid.as_raw()))
        .ok()
        .is_some_and(|stat| {
            stat.rsplit_once(')')
                .and_then(|(_, fields)| fields.split_whitespace().next())
                == Some("T")
        })
}

pub(crate) fn process_exists(pid: Pid) -> bool {
    Path::new(&format!("/proc/{}", pid.as_raw())).exists()
}

pub(crate) fn position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|candidate| candidate == needle)
}

pub(crate) fn last_position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|candidate| candidate == needle)
}

pub(crate) fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|candidate| *candidate == needle)
        .count()
}

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
