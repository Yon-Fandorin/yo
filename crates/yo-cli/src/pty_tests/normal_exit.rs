use std::{
    error::Error,
    io::Write,
    task::{Context, Poll},
};

use yo_tui::{PresentationMode, TerminationEvent, TerminationSource};

use super::support::{
    CHILD_MARKER, ENTER_ALTERNATE_SCREEN, PtyChild, RetainedChatAgent, assert_fullscreen_pair,
    run_fullscreen,
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

// 실제 Linux PTY에서 세 buffered record가 one-at-a-time 순회 중에도 계속 ready로 남아
// 대화를 완성하고, Inline viewport 복구 뒤 그 텍스트가 native scrollback에 남는지 확인한다.
#[test]
fn inline_normal_exit_retains_chat_after_viewport_restoration() {
    const RETAINED: &[u8] = "• YO_INLINE_RETAINED\r\n".as_bytes();
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
        position(&output, b"\x1b[2K") < position(&output, RETAINED),
        "plain chat output must follow viewport restoration:\n{}",
        String::from_utf8_lossy(&output)
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
