use std::{
    error::Error,
    io::Write,
    task::{Context, Poll},
    time::Duration,
};

use nix::{sys::signal::Signal, unistd::Pid};
use yo_tui::{PresentationMode, TerminationEvent, TerminationSource};

use super::support::{
    CHILD_MARKER, ChildReapReceipt, ENTER_ALTERNATE_SCREEN, PendingAgent, PtyChild,
    RetainedChatAgent, assert_fullscreen_pair, run_fullscreen,
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
    agent: &mut RetainedChatAgent,
) -> Result<(), Box<dyn Error>> {
    let outcome = yo_tui::run_with_mode(
        termination,
        agent,
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

// 큰 persistent publication의 capture를 marker 직후 멈춰 writer가 flush를 끝내지 못하게
// 한 상태에서 PTY를 100열로 바꾼다. capture 재개 뒤 post-flush geometry 재검사가 80열에는
// 불가능한 90칸 rule과 새 frame을 입력보다 먼저 그려도 acknowledged Final은 재출력하지 않는다.
#[test]
fn inline_resize_does_not_replay_published_history() {
    const RETAINED: &[u8] = b"YO_INLINE_RETAINED";
    const RESIZED_FRAME_COMPLETE: &[u8] = b"inline\x1b[3A\x1b[3G\x1b[?25h";
    const POST_RESIZE_INPUT_READY: &[u8] = b"}";
    const RESIZED_RULE: &[u8] = concat!(
        "──────────",
        "──────────",
        "──────────",
        "──────────",
        "──────────",
        "──────────",
        "──────────",
        "──────────",
        "──────────"
    )
    .as_bytes();
    let mut child = PtyChild::spawn_with_capture_pause(
        "pty_tests::normal_exit::child_inline_retains_large_chat",
        &[
            RETAINED,
            POST_RESIZE_INPUT_READY,
            RESIZED_RULE,
            RESIZED_FRAME_COMPLETE,
        ],
        0,
    );
    let retained_end = child.wait_until_ready_marker(0);
    child.resize(100, 30);
    child.release_capture();
    let resized_rule_end = child.wait_until_ready_marker_after(2, retained_end);
    let resized_generation_end = child.wait_until_ready_marker_after(3, resized_rule_end);
    child.input.write_all(POST_RESIZE_INPUT_READY).unwrap();
    child.input.flush().unwrap();
    let resized_input_end = child.wait_until_ready_marker_after(1, resized_generation_end);
    child.wait_until_output_reaches(resized_input_end);
    child.input.write_all(&[0x7f, 0x04]).unwrap();
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

// PTY 준비 표식을 기다리다 시간 초과가 발생해도 중첩 테스트 자식을 즉시 회수해 후속
// PTY 테스트와 훅 실행을 가로막는 고아 프로세스를 남기지 않는다.
#[test]
fn readiness_timeout_reaps_the_nested_pty_child() {
    let child = PtyChild::spawn(
        "pty_tests::normal_exit::child_inline_empty_prompt",
        b"YO_MARKER_THAT_NEVER_APPEARS",
    );
    let pid = Pid::from_raw(i32::try_from(child.child.id()).unwrap());

    let started = std::time::Instant::now();
    let error = child
        .wait_until_ready_marker_with_timeout(0, Duration::from_millis(100))
        .expect_err("the fixture marker must remain absent");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "readiness cleanup must remain bounded: {error}"
    );
    assert!(
        matches!(
            error.cleanup(),
            Ok(ChildReapReceipt::Waitpid(nix::sys::wait::WaitStatus::Signaled(
                reaped,
                Signal::SIGKILL,
                _
            ))) if *reaped == pid
        ),
        "a readiness timeout must retain the exact-child waitpid receipt: {error}"
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
    let mut agent = RetainedChatAgent::new();
    run_inline_with_retained_chat(&mut PendingTermination, &mut agent).unwrap();
}

// resize publication 자식은 PTY backpressure를 만들 만큼 큰 한 항목을 발행해 부모가
// post-flush geometry 재검사 전에 크기를 변경할 수 있는 deterministic 경계를 제공한다.
#[test]
#[ignore]
fn child_inline_retains_large_chat() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    let mut agent = RetainedChatAgent::new_with_large_publication();
    run_inline_with_retained_chat(&mut PendingTermination, &mut agent).unwrap();
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
