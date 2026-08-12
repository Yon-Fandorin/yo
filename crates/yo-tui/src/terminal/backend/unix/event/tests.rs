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

use super::UnixEventReader;
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
    pending: VecDeque<Poll<TerminationEvent>>,
}

impl TerminationSource for PendingTermination {
    fn poll_termination(&mut self, _context: &mut Context<'_>) -> Poll<TerminationEvent> {
        self.pending.pop_front().unwrap_or(Poll::Pending)
    }
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

// terminal producer가 등록된 waker를 깨우면 fixed fallback 없이 indefinite owner wait가
// 끝나고 같은 reader에서 준비된 input 한 건을 읽는다.
#[test]
fn producer_wake_preempts_indefinite_wait() {
    let input = WakingEventSource {
        ready: Arc::new(AtomicBool::new(false)),
        spawned: false,
    };
    let mut reader = UnixEventReader::new(input, PendingTermination::default());
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let started = Instant::now();

    assert!(reader.poll_input(&mut context).is_pending());
    reader.wait(None);
    assert_eq!(
        reader.poll_input(&mut context),
        Poll::Ready(Ok(InputEvent::Paste("wake".to_owned())))
    );
    assert!(started.elapsed() < Duration::from_millis(500));
}

// termination readiness는 terminal input consumption과 분리되어 scheduler가 ordinary
// cursor를 시작하기 전에 strict-priority control path를 확인할 수 있다.
#[test]
fn termination_can_be_polled_without_polling_terminal_input() {
    let mut reader = UnixEventReader::new(
        RecordingEventSource {
            ready: VecDeque::from([Err("must not poll")]),
            ..RecordingEventSource::default()
        },
        PendingTermination {
            pending: VecDeque::from([Poll::Ready(TerminationEvent::Requested)]),
        },
    );
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert_eq!(
        reader.poll_termination(&mut context),
        Poll::Ready(TerminationEvent::Requested)
    );
}

// 준비된 terminal event는 다른 source payload와 결합하지 않고 semantic input 한 건으로
// owner-thread scheduler에 전달된다.
#[test]
fn returns_one_ready_terminal_input() {
    let mut reader = UnixEventReader::new(
        RecordingEventSource {
            ready: VecDeque::from([Ok(true)]),
            events: VecDeque::from([Ok(Event::Paste("질문".to_owned()))]),
        },
        PendingTermination::default(),
    );
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert_eq!(
        reader.poll_input(&mut context),
        Poll::Ready(Ok(InputEvent::Paste("질문".to_owned())))
    );
}

// terminal source failure는 pending readiness로 축약되지 않고 scheduler가 관찰할 typed
// input failure로 그대로 남는다.
#[test]
fn preserves_terminal_poll_failure() {
    let mut reader = UnixEventReader::new(
        RecordingEventSource {
            ready: VecDeque::from([Err("poll failed")]),
            ..RecordingEventSource::default()
        },
        PendingTermination::default(),
    );
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert_eq!(
        reader.poll_input(&mut context),
        Poll::Ready(Err(InputReadFailure::Source("poll failed")))
    );
}

// flush 직후 이미 ready인 모든 resize는 blocking 없이 세고 마지막 크기를 반환하며, 그
// 사이의 일반 input은 버리지 않고 원래 순서대로 다음 ordinary poll에 넘긴다.
#[test]
fn post_flush_observation_drains_resizes_and_preserves_ordinary_input() {
    let mut reader = UnixEventReader::new(
        RecordingEventSource {
            ready: VecDeque::from([Ok(true), Ok(true), Ok(true), Ok(false)]),
            events: VecDeque::from([
                Ok(Event::Resize(80, 24)),
                Ok(Event::Paste("kept".to_owned())),
                Ok(Event::Resize(100, 30)),
            ]),
        },
        PendingTermination::default(),
    );
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    let observed = reader.observe_post_flush_resizes(&mut context).unwrap();

    assert_eq!(observed.count, 2);
    assert_eq!(observed.latest, Some(crate::surface::Size::new(100, 30)));
    assert_eq!(
        reader.poll_input(&mut context),
        Poll::Ready(Ok(InputEvent::Paste("kept".to_owned())))
    );
}

// post-flush drain 중 source 오류도 성공적인 geometry 관찰로 축약하지 않아 presenter가
// live ownership을 버리고 기존 input failure 진단을 유지할 수 있다.
#[test]
fn post_flush_observation_preserves_input_failure() {
    let mut reader = UnixEventReader::new(
        RecordingEventSource {
            ready: VecDeque::from([Err("post-flush failure")]),
            ..RecordingEventSource::default()
        },
        PendingTermination::default(),
    );
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert!(matches!(
        reader.observe_post_flush_resizes(&mut context),
        Err(super::PostFlushObservationError::Input(
            InputReadFailure::Source("post-flush failure")
        ))
    ));
}

// 지속적으로 ready인 input producer가 flush 이후 관찰을 무한 loop나 무제한 queue로
// 바꾸지 못한다. 한 세대의 명시적 상한에 도달하면 성공으로 축약하지 않고 typed 오류를
// 반환해 presenter가 live ownership을 폐기하게 한다.
#[test]
fn post_flush_observation_bounds_a_continuously_ready_source() {
    let limit = super::POST_FLUSH_EVENT_LIMIT;
    let mut reader = UnixEventReader::new(
        RecordingEventSource {
            ready: std::iter::repeat_n(Ok(true), limit).collect(),
            events: std::iter::repeat_with(|| Ok(Event::Paste("kept".to_owned())))
                .take(limit)
                .collect(),
        },
        PendingTermination::default(),
    );
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert!(matches!(
        reader.observe_post_flush_resizes(&mut context),
        Err(super::PostFlushObservationError::EventLimitExceeded {
            limit: observed
        }) if observed == limit
    ));
}

// bounded wait는 source를 몰래 poll하지 않고 지정된 deadline이 지난 뒤 caller가 모든
// source readiness를 다시 선택하도록 제어를 반환한다.
#[test]
fn bounded_wait_returns_to_the_scheduler() {
    let reader = UnixEventReader::new(
        RecordingEventSource::default(),
        PendingTermination::default(),
    );

    reader.wait(Some(Duration::ZERO));
}
