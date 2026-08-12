use std::{
    error::Error,
    io::Write,
    task::{Context, Poll},
};

use yo_tui::{PresentationMode, TerminationEvent, TerminationSource};

use super::support::{
    CHILD_MARKER, ENTER_ALTERNATE_SCREEN, PendingAgent, PtyChild, RetainedChatAgent,
    assert_fullscreen_pair, run_fullscreen,
};

struct PendingTermination;

impl TerminationSource for PendingTermination {
    fn poll_termination(&mut self, _context: &mut Context<'_>) -> Poll<TerminationEvent> {
        Poll::Pending
    }
}

fn position(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .rposition(|candidate| candidate == needle)
        .expect("expected bytes must be present")
}

fn run_inline_with_retained_chat(
    termination: &mut impl TerminationSource,
) -> Result<(), Box<dyn Error>> {
    let mut agent = RetainedChatAgent::new();
    let outcome = yo_tui::run_with_mode(
        termination,
        &mut agent,
        PresentationMode::Inline,
        yo_tui::ColorCapability::Unknown,
        yo_tui::MotionPreference::Standard,
    )?;
    match outcome {
        yo_tui::TerminalOutcome::Exited(outcome) => {
            if let Some(output) = outcome.output() {
                super::super::write_session_output(output)?;
            }
        },
        yo_tui::TerminalOutcome::SuspendRequested => {
            return Err("unexpected suspension in retained-chat PTY helper".into());
        },
        _ => return Err("unsupported terminal outcome in retained-chat PTY helper".into()),
    }
    Ok(())
}

// 실제 Linux PTY에서 Final Chat 항목은 active viewport 복구 전에 persistent output으로 한
// 번만 기록되고, 정상 종료 뒤 unpublished suffix가 없어 다시 출력되지 않는다.
#[test]
fn inline_normal_exit_retains_chat_after_viewport_restoration() {
    const RETAINED: &[u8] = "YO_INLINE_RETAINED".as_bytes();
    let mut child = PtyChild::spawn(
        "pty_tests::normal_exit::child_inline_retains_chat",
        b"YO_INLINE_RETAINED",
    );
    child.wait_until_ready();
    child.input.write_all(&[0x04]).unwrap();
    child.input.flush().unwrap();

    let (status, output) = child.finish();
    assert!(
        status.success(),
        "child failed:\n{}",
        String::from_utf8_lossy(&output)
    );
    assert!(
        position(&output, RETAINED) < position(&output, b"\x1b[2K"),
        "persistent Chat output must precede final live-viewport restoration:\n{}",
        String::from_utf8_lossy(&output)
    );
    assert_eq!(
        output
            .windows(RETAINED.len())
            .filter(|candidate| *candidate == RETAINED)
            .count(),
        1,
        "acknowledged Chat output must not be replayed as an exit suffix"
    );
}

// 80x24 실제 Linux PTY에서 비어 있는 Inline Chat은 terminal 전체 24행을 확보하지 않고
// transcript floor, framed prompt와 chrome에 필요한 9행만 live region으로 할당한다.
#[test]
fn inline_empty_prompt_uses_a_compact_live_region() {
    let mut child = PtyChild::spawn(
        "pty_tests::normal_exit::child_inline_empty_prompt",
        b"\x1b[?25l",
    );
    child.wait_until_ready();
    child.input.write_all(&[0x04]).unwrap();
    child.input.flush().unwrap();

    let (status, output) = child.finish();
    assert!(
        status.success(),
        "child failed:\n{}",
        String::from_utf8_lossy(&output)
    );
    assert!(
        output
            .windows(10)
            .any(|window| window == b"\r\n\n\n\n\n\n\n\n\n"),
        "Inline must allocate the compact nine-row live region:\n{}",
        String::from_utf8_lossy(&output)
    );
    assert!(
        !output
            .windows(25)
            .any(|window| window == b"\r\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n"),
        "Inline must not allocate all 24 terminal rows"
    );
}

// 실제 Linux PTY 크기를 바꾸고 SIGWINCH를 전달해도 이미 persistent scrollback으로
// acknowledge한 Final 항목은 다시 출력하지 않고 compact live viewport만 새 폭으로 그린다.
#[test]
fn inline_resize_does_not_replay_published_history() {
    const RETAINED: &[u8] = b"YO_INLINE_RETAINED";
    let mut child = PtyChild::spawn(
        "pty_tests::normal_exit::child_inline_retains_chat",
        b"\x1b[?25l",
    );
    child.wait_until_ready();
    child.wait_until_ready();
    child.resize(100, 30);
    child.wait_until_ready();
    child.input.write_all(&[0x04]).unwrap();
    child.input.flush().unwrap();

    let (status, output) = child.finish();
    assert!(
        status.success(),
        "child failed:\n{}",
        String::from_utf8_lossy(&output)
    );
    assert_eq!(
        output
            .windows(RETAINED.len())
            .filter(|candidate| *candidate == RETAINED)
            .count(),
        1,
        "ordinary resize must not replay acknowledged native history"
    );
}

// 실제 Linux PTY에서 Ctrl+D 정상 종료가 대체 화면과 termios를 모두 원래 상태로 복구한다.
#[test]
fn fullscreen_normal_exit_restores_real_pty() {
    let mut child = PtyChild::spawn(
        "pty_tests::normal_exit::child_fullscreen_normal_exit",
        ENTER_ALTERNATE_SCREEN,
    );
    child.wait_until_ready();
    child.input.write_all(&[0x04]).unwrap();
    child.input.flush().unwrap();

    let (status, output) = child.finish();
    assert!(
        status.success(),
        "child failed:\n{}",
        String::from_utf8_lossy(&output)
    );
    assert_fullscreen_pair(&output);
}

// 부모 테스트가 마련한 PTY 안에서 정상 Ctrl+D 종료 경로만 실행하는 자식 진입점이다.
#[test]
#[ignore]
fn child_fullscreen_normal_exit() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    run_fullscreen(&mut PendingTermination).unwrap();
}

// 부모 테스트가 마련한 PTY에서 deterministic 일반 대화를 그린 뒤 Inline 정상 종료한다.
#[test]
#[ignore]
fn child_inline_retains_chat() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    run_inline_with_retained_chat(&mut PendingTermination).unwrap();
}

// 부모 테스트가 마련한 PTY에서 semantic Chat 없이 compact Inline prompt만 그린다.
#[test]
#[ignore]
fn child_inline_empty_prompt() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    let mut agent = PendingAgent;
    yo_tui::run_with_mode(
        &mut PendingTermination,
        &mut agent,
        PresentationMode::Inline,
        yo_tui::ColorCapability::Unknown,
        yo_tui::MotionPreference::Standard,
    )
    .unwrap();
}
