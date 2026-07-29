#![cfg(target_os = "linux")]

#[path = "terminal_matrix/local_tmux.rs"]
mod local_tmux;
#[path = "terminal_matrix/ssh.rs"]
mod ssh;
#[path = "terminal_matrix/support.rs"]
mod support;
