use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use super::{EXIT_MARKER, ShellJob, TmuxSession, assert_tmux_server_absent};
use crate::support::shell_quote;

fn make_editor(session: &TmuxSession, body: &str) -> (PathBuf, PathBuf) {
    let editor = session.state_root.join("editor.sh");
    let observed_path = session.state_root.join("editor-path");
    fs::write(
        &editor,
        format!(
            "#!/bin/sh\nprintf '%s' \"$1\" > {}\n{body}\n",
            shell_quote(&observed_path)
        ),
    )
    .unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o700)).unwrap();
    (editor, observed_path)
}

fn assert_editor_round_trip(option: &str, alternate_screen: bool, succeed: bool) {
    let session = TmuxSession::create();
    let body = if succeed {
        "printf 'edited first\\nedited second' > \"$1\""
    } else {
        "exit 17"
    };
    let (editor, observed_path) = make_editor(&session, body);
    let job = session.run_mode_under_shell_with_editor(option, alternate_screen, Some(&editor));

    session.send_literal("original draft");
    session.wait_for_draft(&["original draft"], &["edited first"]);
    session.run_tmux(&["send-keys", "-t", &session.name, "C-g"]);
    session.wait_until(super::COMMAND_READY_TIMEOUT, |state| {
        !state.dead && observed_path.exists()
    });
    if succeed {
        session.wait_for_draft(&["edited first", "edited second"], &["original draft"]);
    } else {
        session.wait_for_draft(&["original draft"], &["edited first"]);
    }
    let temporary = fs::read_to_string(&observed_path).unwrap();
    assert!(
        !Path::new(&temporary).exists(),
        "external editor temporary draft remained after reentry"
    );
    assert_eq!(session.shell_child(), Some(job.yo_pid));
    session.assert_no_inference();

    finish_editor_session(session, job);
}

fn finish_editor_session(session: TmuxSession, job: ShellJob) {
    session.clear_draft();
    session.send_empty_ctrl_d();
    session.wait_for_yo_exit_then_exit_shell(&job);
    let exit = session.wait_for_clean_exit();
    let output = session.captured_text();
    assert_eq!(exit.status, Some(0));
    assert!(output.contains(&format!("{EXIT_MARKER}:0")));
    let name = session.name.clone();
    let socket = session.socket.clone();
    let state_root = session.state_root.clone();
    drop(session);
    assert_tmux_server_absent(&socket, &name);
    assert!(
        !state_root.exists(),
        "isolated Yo state remained after cleanup"
    );
}

// 실제 tmux Inline에서 Ctrl+G 편집 후 전송 없이 초안이 복원되고 터미널이 다시 입력받는지 확인한다.
#[test]
#[ignore = "requires local tmux and a compatible installed Codex"]
fn local_tmux_inline_external_editor_round_trip() {
    assert_editor_round_trip("--inline", false, true);
}

// 실제 tmux Fullscreen에서 Ctrl+G 편집 후 전송 없이 초안이 복원되는지 확인한다.
#[test]
#[ignore = "requires local tmux and a compatible installed Codex"]
fn local_tmux_fullscreen_external_editor_round_trip() {
    assert_editor_round_trip("--fullscreen", true, true);
}

// 실패한 외부 편집기가 기존 초안을 지우지 않고 Yo 입력 화면으로 복귀하는지 확인한다.
#[test]
#[ignore = "requires local tmux and a compatible installed Codex"]
fn local_tmux_inline_failed_external_editor_retains_draft() {
    assert_editor_round_trip("--inline", false, false);
}

// 편집기가 시작 직후 TTY를 읽어도 전경 인계 경쟁으로 중지되지 않는지 확인한다.
#[test]
#[ignore = "requires local tmux and a compatible installed Codex"]
fn local_tmux_editor_reads_terminal_immediately() {
    let session = TmuxSession::create();
    let (editor, observed_path) = make_editor(
        &session,
        "printf 'YO_EDITOR_READ_READY\\n'\nIFS= read -r answer\nprintf 'typed:%s' \"$answer\" > \"$1\"",
    );
    let job = session.run_mode_under_shell_with_editor("--inline", false, Some(&editor));
    session.send_literal("initial draft");
    session.wait_for_draft(&["initial draft"], &[]);
    session.run_tmux(&["send-keys", "-t", &session.name, "C-g"]);
    session.wait_until(super::COMMAND_READY_TIMEOUT, |state| {
        !state.dead && session.captured_text().contains("YO_EDITOR_READ_READY")
    });
    session.send_literal("go");
    session.send_enter();
    session.wait_for_draft(&["typed:go"], &["initial draft"]);
    let temporary = fs::read_to_string(&observed_path).unwrap();
    assert!(!Path::new(&temporary).exists());
    session.assert_no_inference();
    finish_editor_session(session, job);
}

// 편집기에 보낸 Ctrl+C가 편집기 프로세스만 중단하고 Yo와 기존 초안을 살려 두는지 확인한다.
#[test]
#[ignore = "requires local tmux and a compatible installed Codex"]
fn local_tmux_editor_ctrl_c_retains_yo_and_draft() {
    assert_editor_signal_recovers("C-c");
}

// 중지된 편집기를 기다리며 멈추지 않고 원래 초안으로 돌아오는지 확인한다.
#[test]
#[ignore = "requires local tmux and a compatible installed Codex"]
fn local_tmux_editor_ctrl_z_retains_yo_and_draft() {
    assert_editor_signal_recovers("C-z");
}

fn assert_editor_signal_recovers(key: &str) {
    let session = TmuxSession::create();
    let (editor, observed_path) = make_editor(
        &session,
        "trap 'exit 130' INT\nprintf 'YO_EDITOR_WAITING\\n'\nwhile :; do sleep 1; done",
    );
    let job = session.run_mode_under_shell_with_editor("--inline", false, Some(&editor));
    session.send_literal("draft survives editor interrupt");
    session.wait_for_draft(&["draft survives editor interrupt"], &[]);
    session.run_tmux(&["send-keys", "-t", &session.name, "C-g"]);
    session.wait_until(super::COMMAND_READY_TIMEOUT, |state| {
        !state.dead && session.captured_text().contains("YO_EDITOR_WAITING")
    });
    session.run_tmux(&["send-keys", "-t", &session.name, key]);
    session.wait_for_draft(&["draft survives editor interrupt"], &[]);
    let temporary = fs::read_to_string(&observed_path).unwrap();
    assert!(!Path::new(&temporary).exists());
    assert_eq!(session.shell_child(), Some(job.yo_pid));
    session.assert_no_inference();
    finish_editor_session(session, job);
}
