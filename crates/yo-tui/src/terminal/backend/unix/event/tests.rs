use std::{collections::VecDeque, time::Duration};

use crossterm::event::Event;

use super::{UnixEvent, UnixEventReader};
use crate::{
    input::event::InputEvent,
    terminal::backend::unix::{
        input::{EventSource, InputReadFailure, InputReader},
        signal::{SignalSource, TerminationSignal},
    },
};

#[derive(Default)]
struct RecordingEventSource {
    ready: VecDeque<Result<bool, &'static str>>,
    events: VecDeque<Result<Event, &'static str>>,
}

impl EventSource for RecordingEventSource {
    type Error = &'static str;

    fn poll(&mut self, _timeout: Duration) -> Result<bool, Self::Error> {
        self.ready.pop_front().unwrap_or(Ok(false))
    }

    fn read(&mut self) -> Result<Event, Self::Error> {
        self.events.pop_front().unwrap_or(Err("missing event"))
    }
}

#[derive(Default)]
struct PendingSignals {
    pending: VecDeque<Option<TerminationSignal>>,
}

impl SignalSource for PendingSignals {
    fn pending(&mut self) -> Option<TerminationSignal> {
        self.pending.pop_front().flatten()
    }
}

fn reader(
    input: RecordingEventSource,
    pending: impl IntoIterator<Item = Option<TerminationSignal>>,
) -> UnixEventReader<RecordingEventSource, PendingSignals> {
    UnixEventReader::new(
        InputReader::new(input),
        PendingSignals {
            pending: pending.into_iter().collect(),
        },
    )
}

// 이미 도착한 종료 신호는 terminal input을 기다리기 전에 우선 전달한다.
#[test]
fn pending_signal_preempts_terminal_polling() {
    let mut reader = reader(
        RecordingEventSource {
            ready: VecDeque::from([Err("must not poll")]),
            ..RecordingEventSource::default()
        },
        [Some(TerminationSignal::Terminate)],
    );

    assert_eq!(
        reader.next(Duration::from_secs(1)).unwrap(),
        UnixEvent::Terminate(TerminationSignal::Terminate)
    );
}

// 준비된 terminal event는 같은 owner thread에서 semantic input으로 반환한다.
#[test]
fn returns_ready_terminal_input() {
    let input = RecordingEventSource {
        ready: VecDeque::from([Ok(true)]),
        events: VecDeque::from([Ok(Event::Paste("질문".to_owned()))]),
    };
    let mut reader = reader(input, [None]);
    let timeout = Duration::from_millis(25);

    assert_eq!(
        reader.next(timeout).unwrap(),
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
        [None, Some(TerminationSignal::Hangup)],
    );

    assert_eq!(
        reader.next(Duration::from_millis(25)).unwrap(),
        UnixEvent::Terminate(TerminationSignal::Hangup)
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
        [None, Some(TerminationSignal::Terminate)],
    );

    assert_eq!(
        reader.next(Duration::from_millis(25)).unwrap(),
        UnixEvent::Terminate(TerminationSignal::Terminate)
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
        [None, Some(TerminationSignal::Quit)],
    );

    assert_eq!(
        reader.next(Duration::from_millis(25)).unwrap(),
        UnixEvent::Terminate(TerminationSignal::Quit)
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
        [None, None],
    );

    assert_eq!(
        reader.next(Duration::from_millis(25)).unwrap(),
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
        [None],
    );

    assert_eq!(
        reader.next(Duration::from_millis(25)),
        Err(InputReadFailure::Source("poll failed"))
    );
}
