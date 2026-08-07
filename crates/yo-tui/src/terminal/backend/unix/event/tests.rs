use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    thread::{self, Thread},
    time::{Duration, Instant},
};

use crossterm::event::Event;

use super::{UnixEvent, UnixEventReader};
use crate::{
    input::event::InputEvent,
    runner::{TerminationEvent, TerminationSource},
    terminal::backend::unix::input::{EventSource, InputReadFailure},
};

#[derive(Default)]
struct RecordingEventSource {
    ready: VecDeque<Result<bool, &'static str>>,
    events: VecDeque<Result<Event, &'static str>>,
}

impl EventSource for RecordingEventSource {
    type Error = &'static str;

    fn poll_event(&mut self, _context: &mut Context<'_>) -> Poll<Result<Event, Self::Error>> {
        match self.ready.pop_front().unwrap_or(Ok(false)) {
            Ok(true) => Poll::Ready(self.events.pop_front().unwrap_or(Err("missing event"))),
            Ok(false) => Poll::Pending,
            Err(error) => Poll::Ready(Err(error)),
        }
    }
}

#[derive(Default)]
struct PendingTermination {
    pending: VecDeque<TerminationEvent>,
}

impl TerminationSource for PendingTermination {
    fn poll_termination(&mut self) -> TerminationEvent {
        self.pending.pop_front().unwrap_or(TerminationEvent::None)
    }
}

fn reader(
    input: RecordingEventSource,
    pending: impl IntoIterator<Item = TerminationEvent>,
) -> UnixEventReader<RecordingEventSource, PendingTermination> {
    UnixEventReader::new(
        input,
        PendingTermination {
            pending: pending.into_iter().collect(),
        },
    )
}

fn next(
    reader: &mut UnixEventReader<RecordingEventSource, PendingTermination>,
    timeout: Duration,
) -> Result<UnixEvent, InputReadFailure<&'static str>> {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    reader.next(timeout, &mut context)
}

struct ThreadWake(Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

struct WakingEventSource {
    ready: Arc<AtomicBool>,
    spawned: bool,
}

impl EventSource for WakingEventSource {
    type Error = &'static str;

    fn poll_event(&mut self, context: &mut Context<'_>) -> Poll<Result<Event, Self::Error>> {
        if self.ready.load(Ordering::Acquire) {
            return Poll::Ready(Ok(Event::Paste("wake".to_owned())));
        }
        if !self.spawned {
            self.spawned = true;
            let ready = Arc::clone(&self.ready);
            let waker = context.waker().clone();
            thread::spawn(move || {
                ready.store(true, Ordering::Release);
                waker.wake();
            });
        }
        Poll::Pending
    }
}

// terminal producer의 wake는 긴 fallback timeout을 기다리지 않고 owner thread를 즉시 재개합니다.
#[test]
fn producer_wake_preempts_bounded_fallback_wait() {
    let input = WakingEventSource {
        ready: Arc::new(AtomicBool::new(false)),
        spawned: false,
    };
    let mut reader = UnixEventReader::new(input, PendingTermination::default());
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let started = Instant::now();

    assert_eq!(
        reader.next(Duration::from_secs(1), &mut context).unwrap(),
        UnixEvent::Input(InputEvent::Paste("wake".to_owned()))
    );
    assert!(started.elapsed() < Duration::from_millis(500));
}

// 이미 도착한 종료 신호는 terminal input을 기다리기 전에 우선 전달한다.
#[test]
fn pending_signal_preempts_terminal_polling() {
    let mut reader = reader(
        RecordingEventSource {
            ready: VecDeque::from([Err("must not poll")]),
            ..RecordingEventSource::default()
        },
        [TerminationEvent::Requested],
    );

    assert_eq!(
        next(&mut reader, Duration::from_secs(1)).unwrap(),
        UnixEvent::Terminate
    );
}

// 준비된 terminal event는 같은 owner thread에서 semantic input으로 반환한다.
#[test]
fn returns_ready_terminal_input() {
    let input = RecordingEventSource {
        ready: VecDeque::from([Ok(true)]),
        events: VecDeque::from([Ok(Event::Paste("질문".to_owned()))]),
    };
    let mut reader = reader(input, [TerminationEvent::None]);
    let timeout = Duration::from_millis(25);

    assert_eq!(
        next(&mut reader, timeout).unwrap(),
        UnixEvent::Input(InputEvent::Paste("질문".to_owned()))
    );
}

// input poll 동안 도착한 종료 신호도 idle을 반환하기 전에 다시 확인한다.
#[test]
fn observes_signal_arriving_during_terminal_poll() {
    let mut reader = reader(
        RecordingEventSource {
            ready: VecDeque::from([Ok(false)]),
            ..RecordingEventSource::default()
        },
        [TerminationEvent::None, TerminationEvent::Requested],
    );

    assert_eq!(
        next(&mut reader, Duration::from_millis(25)).unwrap(),
        UnixEvent::Terminate
    );
}

// input과 종료 신호가 함께 준비되어도 종료를 먼저 전달해 state mutation을 막는다.
#[test]
fn post_poll_signal_preempts_ready_terminal_input() {
    let mut reader = reader(
        RecordingEventSource {
            ready: VecDeque::from([Ok(true)]),
            events: VecDeque::from([Ok(Event::Paste("discarded".to_owned()))]),
        },
        [TerminationEvent::None, TerminationEvent::Requested],
    );

    assert_eq!(
        next(&mut reader, Duration::from_millis(25)).unwrap(),
        UnixEvent::Terminate
    );
}

// poll 실패와 종료 신호가 경쟁하면 terminal 복구를 위한 종료가 오류보다 우선한다.
#[test]
fn post_poll_signal_preempts_terminal_failure() {
    let mut reader = reader(
        RecordingEventSource {
            ready: VecDeque::from([Err("poll failed")]),
            ..RecordingEventSource::default()
        },
        [TerminationEvent::None, TerminationEvent::Requested],
    );

    assert_eq!(
        next(&mut reader, Duration::from_millis(25)).unwrap(),
        UnixEvent::Terminate
    );
}

// input과 signal이 모두 없으면 제한된 대기 뒤 명시적인 idle을 반환한다.
#[test]
fn returns_idle_after_bounded_poll() {
    let mut reader = reader(
        RecordingEventSource {
            ready: VecDeque::from([Ok(false)]),
            ..RecordingEventSource::default()
        },
        [TerminationEvent::None, TerminationEvent::None],
    );

    assert_eq!(
        next(&mut reader, Duration::from_millis(25)).unwrap(),
        UnixEvent::Idle
    );
}

// polling 실패는 신호 부재나 idle로 숨기지 않고 입력 오류로 보존한다.
#[test]
fn preserves_terminal_poll_failure() {
    let mut reader = reader(
        RecordingEventSource {
            ready: VecDeque::from([Err("poll failed")]),
            ..RecordingEventSource::default()
        },
        [TerminationEvent::None, TerminationEvent::None],
    );

    assert_eq!(
        next(&mut reader, Duration::from_millis(25)),
        Err(InputReadFailure::Source("poll failed"))
    );
}
