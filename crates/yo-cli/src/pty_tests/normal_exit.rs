use std::{
    error::Error,
    io::Write,
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Context, Poll},
    time::Duration,
};

use nix::sys::signal::Signal;
use yo_tui::{PresentationMode, TerminationEvent, TerminationSource};

use super::support::{
    CHILD_MARKER, ChildReapReceipt, ENTER_ALTERNATE_SCREEN, PendingAgent, PtyChild,
    ReadinessWaitFailure, RetainedChatAgent, assert_child_is_gone, assert_fullscreen_pair,
    run_fullscreen,
};

type ConsumingReadinessWait =
    fn(PtyChild, usize, Duration) -> Result<(PtyChild, usize), ReadinessWaitFailure>;

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

// 실제 Unix PTY에서 Final Chat 항목은 active viewport 복구 전에 persistent output으로 한
// 번만 기록되고, 정상 종료 뒤 unpublished suffix가 없어 다시 출력되지 않는다.
#[test]
fn inline_normal_exit_retains_chat_after_viewport_restoration() {
    const RETAINED: &[u8] = "YO_INLINE_RETAINED".as_bytes();
    let mut child = PtyChild::spawn(
        "pty_tests::normal_exit::child_inline_retains_chat",
        b"YO_INLINE_RETAINED",
    );
    child.wait_until_ready();
    child.input().write_all(&[0x04]).unwrap();
    child.input().flush().unwrap();

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

// 80x24 실제 Unix PTY에서 비어 있는 Inline Chat은 terminal 전체 24행을 확보하지 않고
// transcript floor, framed prompt와 chrome에 필요한 9행만 live region으로 할당한다.
#[test]
fn inline_empty_prompt_uses_a_compact_live_region() {
    let mut child = PtyChild::spawn(
        "pty_tests::normal_exit::child_inline_empty_prompt",
        b"\x1b[?25l",
    );
    child.wait_until_ready();
    child.input().write_all(&[0x04]).unwrap();
    child.input().flush().unwrap();

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
    child.input().write_all(POST_RESIZE_INPUT_READY).unwrap();
    child.input().flush().unwrap();
    let resized_input_end = child.wait_until_ready_marker_after(1, resized_generation_end);
    child.wait_until_output_reaches(resized_input_end);
    child.input().write_all(&[0x7f, 0x04]).unwrap();
    child.input().flush().unwrap();

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

// 준비 표식이 제한 시간 안에 나타나면 consuming wait는 같은 PID의 live owner와 실제
// 표식 끝 offset을 돌려주며, 반환 owner로 입력·정상 종료·termios 복구·단일 reap을 마친다.
#[test]
fn consuming_readiness_wait_returns_the_live_owner_on_success() {
    const RETAINED: &[u8] = b"YO_INLINE_RETAINED";
    let _: ConsumingReadinessWait = PtyChild::wait_until_ready_marker_with_timeout;
    let child = PtyChild::spawn(
        "pty_tests::normal_exit::child_inline_retains_chat",
        RETAINED,
    );
    let pid = child.pid();

    let (mut child, marker_offset) = child
        .wait_until_ready_marker_with_timeout(0, Duration::from_secs(5))
        .unwrap_or_else(|error| panic!("the fixture marker must appear: {error}"));
    assert_eq!(
        child.pid(),
        pid,
        "the successful wait must return its owner"
    );
    child.input().write_all(&[0x04]).unwrap();
    child.input().flush().unwrap();

    let (status, output) = child.finish();
    assert!(
        status.success(),
        "child failed:\n{}",
        String::from_utf8_lossy(&output)
    );
    assert_eq!(
        marker_offset,
        position(&output, RETAINED) + RETAINED.len(),
        "the successful wait must return the observed marker's end offset"
    );
    assert_child_is_gone(pid);
}

// 이미 한 표식을 관측한 PTY에서 다음 표식이 사라져도 consuming timeout은 8KiB 이하
// 출력을 보존하고 exact SIGKILL receipt를 돌려준 뒤 owner 없이 반환해 child를 남기지 않는다.
#[test]
fn readiness_timeout_consumes_and_reaps_the_exact_child() {
    const RETAINED: &[u8] = b"YO_INLINE_RETAINED";
    let mut child = PtyChild::spawn_with_ready_markers(
        "pty_tests::normal_exit::child_inline_retains_chat",
        &[RETAINED, b"YO_MARKER_THAT_NEVER_APPEARS"],
    );
    child.wait_until_ready_marker(0);
    let pid = child.pid();

    let started = std::time::Instant::now();
    let error = child
        .wait_until_ready_marker_with_timeout(1, Duration::from_millis(100))
        .err()
        .expect("the fixture marker must remain absent");
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
    assert!(
        error
            .output()
            .windows(RETAINED.len())
            .any(|candidate| candidate == RETAINED),
        "the failure must retain output observed before the timeout: {error}"
    );
    assert!(
        error.output().len() <= 8 * 1024,
        "the readiness diagnostic must retain only the bounded output window"
    );
    assert_child_is_gone(pid);
}

// Parent assertion unwind는 finish 호출 여부와 무관하게 Drop owner가 실행 중인 nested
// child를 kill·reap하고 capture thread를 닫아 다음 PTY test에 남기지 않습니다.
#[test]
fn panic_unwind_reaps_a_running_pty_child() {
    let mut pid = None;
    let panic = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut child = PtyChild::spawn(
            "pty_tests::normal_exit::child_inline_empty_prompt",
            b"\x1b[?25l",
        );
        child.wait_until_ready();
        pid = Some(child.pid());
        panic!("injected parent assertion failure");
    }));

    assert!(panic.is_err());
    assert_child_is_gone(pid.unwrap());
}

// Child spawn 직후 주입한 setup panic은 임시 owner가 1초 안에 exact child를 kill·reap하고,
// 최종 fallible 단계인 capture spawn 전이므로 capture thread를 시작하거나 detach하지 않습니다.
#[test]
fn post_spawn_setup_panic_reaps_before_capture_starts() {
    let (pid_tx, pid_rx) = mpsc::channel();
    let capture_started = Arc::new(AtomicBool::new(false));
    let started = std::time::Instant::now();
    let panic = std::panic::catch_unwind(AssertUnwindSafe({
        let capture_started = Arc::clone(&capture_started);
        move || {
            let _child = PtyChild::spawn_with_injected_post_spawn_failure(
                "pty_tests::normal_exit::child_inline_empty_prompt",
                b"\x1b[?25l",
                pid_tx,
                capture_started,
            );
        }
    }));

    assert!(panic.is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
    let pid = pid_rx
        .recv_timeout(Duration::from_millis(100))
        .expect("the failure checkpoint must publish the exact child PID");
    assert_child_is_gone(pid);
    assert!(!capture_started.load(Ordering::Acquire));
}

// Capture가 marker 뒤 명시적 release를 기다리는 중에 unwind하면 Drop이 child를 먼저
// kill·reap하고 pause를 release한 뒤 PTY를 닫고 bounded join해 두 owner를 모두 회수합니다.
#[test]
fn panic_unwind_releases_a_paused_pty_capture() {
    let mut pid = None;
    let panic = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut child = PtyChild::spawn_with_capture_pause(
            "pty_tests::normal_exit::child_inline_retains_large_chat",
            &[b"YO_INLINE_RETAINED"],
            0,
        );
        child.wait_until_ready();
        pid = Some(child.pid());
        panic!("injected parent assertion failure while capture is paused");
    }));

    assert!(panic.is_err());
    assert_child_is_gone(pid.unwrap());
}

// 실제 Unix PTY에서 Ctrl+D 정상 종료가 대체 화면과 termios를 모두 원래 상태로 복구한다.
#[test]
fn fullscreen_normal_exit_restores_real_pty() {
    let mut child = PtyChild::spawn(
        "pty_tests::normal_exit::child_fullscreen_normal_exit",
        ENTER_ALTERNATE_SCREEN,
    );
    child.wait_until_ready();
    child.input().write_all(&[0x04]).unwrap();
    child.input().flush().unwrap();

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
