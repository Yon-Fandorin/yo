//! Deterministic output-clock tests: the real appearance, diff, ANSI renderer,
//! and limiter write to a timestamped terminal sink with a simulated I/O cost.
use std::{
    cell::{Cell, RefCell},
    io::{self, Write},
    rc::Rc,
    time::{Duration, Instant},
};

use super::{next_motion_deadline, request_due_motion};
use crate::{
    appearance::AppearanceState,
    runner::frame::{FrameRateLimit, FrameRequest, FrameScheduler},
    surface::{Grapheme, Point, Rect, Size, Style, Surface},
    terminal::mode::fullscreen::{FullscreenRenderer, FullscreenViewport},
};

type OutputRecords = Rc<RefCell<Vec<(Instant, Vec<u8>)>>>;

struct TimedWriter {
    clock: Rc<Cell<Instant>>,
    writes: OutputRecords,
    cost: Duration,
}

impl Write for TimedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.clock.set(self.clock.get() + self.cost);
        self.writes
            .borrow_mut()
            .push((self.clock.get(), bytes.to_vec()));
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// 실제 ANSI 출력 경계에서 60/120fps, 입력 유무, 고정 I/O 지연을 조합해 점 이동 간격과
// FPS 상한을 함께 확인한다. 단순 marker index 계산만 검증하는 테스트가 아니다.
#[test]
fn marker_output_is_even_under_frame_limits_and_input_redraws() {
    for limit in [FrameRateLimit::Fps60, FrameRateLimit::Fps120] {
        for input_interval in [None, Some(Duration::from_millis(7))] {
            for cost in [Duration::ZERO, Duration::from_millis(2)] {
                let epoch = Instant::now();
                let clock = Rc::new(Cell::new(epoch));
                let writes = Rc::new(RefCell::new(Vec::new()));
                let mut renderer = FullscreenRenderer::new(TimedWriter {
                    clock: clock.clone(),
                    writes: writes.clone(),
                    cost,
                });
                let mut viewport = FullscreenViewport::default();
                let pin = AppearanceState::default().pin();
                let mut scheduler = FrameScheduler::new(limit);
                let mut previous = None;
                let mut motion = None;
                let mut input = input_interval.map(|interval| epoch + interval);
                scheduler.request(FrameRequest::Immediate);
                for _ in 0..3000 {
                    let now = clock.get();
                    request_due_motion(&mut scheduler, true, &mut motion, now);
                    if input.is_some_and(|deadline| now >= deadline) {
                        scheduler.request(FrameRequest::Coalesced);
                        input = input_interval.map(|interval| now + interval);
                    }
                    if scheduler.is_due(now) {
                        let elapsed = now.duration_since(epoch);
                        let frame = pin.snapshot().activity_motion_frame(elapsed);
                        let mut surface = Surface::new(Size::new(2, 1)).unwrap();
                        surface
                            .view(Rect::new(Point::new(0, 0), surface.size()))
                            .unwrap()
                            .write(
                                Point::new(0, 0),
                                Grapheme::try_from(frame.marker()).unwrap(),
                                Style::default(),
                            );
                        let pending = viewport
                            .begin_frame(surface.size(), Point::new(1, 0))
                            .unwrap();
                        renderer
                            .render(pending, previous.as_ref(), &surface)
                            .unwrap();
                        previous = Some(surface);
                        let marker = next_motion_deadline(epoch, elapsed, frame.marker_interval());
                        motion = [next_motion_deadline(epoch, elapsed, frame.period()), marker]
                            .into_iter()
                            .flatten()
                            .min();
                        scheduler.reserve_marker(marker);
                        scheduler.rendered_with_cost(now, clock.get());
                    }
                    if clock.get().duration_since(epoch) >= Duration::from_secs(3) {
                        break;
                    }
                    let next = [motion, input, scheduler.deadline(clock.get())]
                        .into_iter()
                        .flatten()
                        .min()
                        .unwrap();
                    clock.set(next.max(clock.get()));
                }
                let records = writes.borrow();
                assert!(!records.is_empty());
                for pair in records.windows(2) {
                    assert!(pair[1].0.duration_since(pair[0].0) >= limit.interval());
                }
                let transitions: Vec<_> = records
                    .iter()
                    .filter_map(|(time, bytes)| {
                        String::from_utf8_lossy(bytes)
                            .chars()
                            .find(|c| ('\u{2800}'..='\u{28ff}').contains(c))
                            .map(|marker| (*time, marker))
                    })
                    .collect();
                assert!(
                    transitions.len() >= 20,
                    "preview must continue over multiple revolutions"
                );
                let markers = ['⠋', '⠙', '⠸', '⠴', '⠦', '⠇'];
                for (index, pair) in transitions.windows(2).enumerate() {
                    assert_eq!(pair[0].1, markers[index % 6]);
                    assert_eq!(
                        pair[1].0.duration_since(pair[0].0),
                        Duration::from_nanos(800_000_000 / 6),
                        "{limit:?}, input={input_interval:?}, cost={cost:?}, transition={index}"
                    );
                }
            }
        }
    }
}
