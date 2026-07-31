use super::{EXIT_MARKER, TmuxSession, assert_tmux_server_absent};

fn assert_repeated_suspend_resume(option: &str, alternate_screen: bool) {
    let session = TmuxSession::create();
    let job = session.run_mode_under_shell(option, alternate_screen);

    for _ in 0..2 {
        session.send_ctrl_z();
        session.wait_for_suspension_then_foreground(&job);
        session.wait_for_mode(alternate_screen);
    }

    session.send_empty_ctrl_d();
    session.wait_for_yo_exit_then_exit_shell(&job);
    let exit = session.wait_for_clean_exit();
    let output = session.captured_text();

    assert_eq!(exit.status, Some(0));
    assert!(output.contains(&format!("{EXIT_MARKER}:0")));
    let session_name = session.name.clone();
    let socket = session.socket.clone();
    drop(session);
    assert_tmux_server_absent(&socket, &session_name);
}

// 실제 Unix tmux의 Inline을 두 번 연속 Ctrl+Z로 중지할 때마다 셸의 원래 termios로
// 돌아오고, `fg` 뒤에는 같은 앱 세션이 새 terminal generation을 획득해 다시 입력받는지 확인한다.
#[test]
#[ignore = "requires local tmux and a compatible installed Codex"]
fn local_tmux_inline_repeated_suspend_resume_restores_each_generation() {
    assert_repeated_suspend_resume("--inline", false);
}

// 실제 Unix tmux의 Fullscreen을 두 번 연속 Ctrl+Z로 중지할 때마다 alternate screen을
// 반납하고 셸 termios를 복구하며, `fg` 뒤에는 Fullscreen을 다시 획득해 정상 종료하는지 확인한다.
#[test]
#[ignore = "requires local tmux and a compatible installed Codex"]
fn local_tmux_fullscreen_repeated_suspend_resume_restores_each_generation() {
    assert_repeated_suspend_resume("--fullscreen", true);
}
