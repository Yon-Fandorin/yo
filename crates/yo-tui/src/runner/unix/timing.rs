use std::time::{Duration, Instant};

use crate::runner::frame::{FrameRequest, FrameScheduler};

pub(super) const WORKER_RETRY_INTERVAL: Duration = Duration::from_millis(10);

pub(super) fn next_motion_deadline(
    epoch: Instant,
    elapsed: Duration,
    period: Option<Duration>,
) -> Option<Instant> {
    let period = period?;
    if period.is_zero() {
        return None;
    }
    let remainder = elapsed.as_nanos() % period.as_nanos();
    let remainder = Duration::new(
        u64::try_from(remainder / 1_000_000_000).ok()?,
        u32::try_from(remainder % 1_000_000_000).ok()?,
    );
    let current_tick_start = elapsed.checked_sub(remainder)?;
    epoch.checked_add(current_tick_start.checked_add(period)?)
}

pub(super) fn wait_timeout(
    base: Option<Duration>,
    motion_deadline: Option<Instant>,
    frame_deadline: Option<Instant>,
) -> Option<Duration> {
    wait_timeout_at(base, motion_deadline, frame_deadline, Instant::now())
}

pub(super) fn wait_timeout_at(
    base: Option<Duration>,
    motion_deadline: Option<Instant>,
    frame_deadline: Option<Instant>,
    now: Instant,
) -> Option<Duration> {
    [motion_deadline, frame_deadline]
        .into_iter()
        .flatten()
        .map(|deadline| deadline.saturating_duration_since(now))
        .fold(base, |timeout, deadline| {
            Some(timeout.map_or(deadline, |current| current.min(deadline)))
        })
}

pub(super) fn request_due_motion(
    frames: &mut FrameScheduler,
    frame_visible: bool,
    motion_deadline: &mut Option<Instant>,
    now: Instant,
) {
    if frame_visible && motion_deadline.is_some_and(|deadline| now >= deadline) {
        frames.request(FrameRequest::Coalesced);
        *motion_deadline = None;
    }
}

#[cfg(test)]
mod motion_tests {
    use std::time::{Duration, Instant};

    use super::{next_motion_deadline, request_due_motion, wait_timeout_at};
    use crate::runner::frame::{FrameRateLimit, FrameRequest, FrameScheduler};

    // backpressure와 frame·motion 마감이 모두 없으면 주기적 poll 없이 무기한 대기합니다.
    #[test]
    fn idle_without_deadlines_has_no_timeout() {
        assert_eq!(wait_timeout_at(None, None, None, Instant::now()), None);
    }

    // 10ms worker 재시도보다 4ms 뒤 motion 마감이 더 가까우면 실제 sleep 없이도
    // scheduler가 정확히 4ms를 선택함을 결정적으로 확인한다.
    #[test]
    fn nearer_motion_deadline_shortens_the_base_wait() {
        let now = Instant::now();

        assert_eq!(
            wait_timeout_at(
                Some(Duration::from_millis(10)),
                now.checked_add(Duration::from_millis(4)),
                None,
                now,
            ),
            Some(Duration::from_millis(4))
        );
    }

    // frame 마감은 backpressure 재시도와 motion 마감보다 가까운 경우 owner wait의
    // 최솟값으로 선택되어 ordinary coalescing 경계를 넘지 않습니다.
    #[test]
    fn nearer_frame_deadline_shortens_the_combined_wait() {
        let now = Instant::now();

        assert_eq!(
            wait_timeout_at(
                Some(Duration::from_millis(10)),
                now.checked_add(Duration::from_millis(7)),
                now.checked_add(Duration::from_millis(3)),
                now,
            ),
            Some(Duration::from_millis(3))
        );
    }

    // 이미 지난 deadline은 음수 duration으로 변환되지 않고 zero로 포화되어 즉시
    // 재선택하게 하며, 현재 시각 이후의 다른 마감값을 잘못 기다리지 않습니다.
    #[test]
    fn overdue_deadlines_saturate_to_zero_wait() {
        let now = Instant::now();
        let past = now.checked_sub(Duration::from_millis(1)).unwrap();

        assert_eq!(
            wait_timeout_at(Some(Duration::from_millis(10)), Some(past), None, now),
            Some(Duration::ZERO)
        );
        assert_eq!(
            wait_timeout_at(Some(Duration::from_millis(10)), None, Some(past), now),
            Some(Duration::ZERO)
        );
    }

    // 16ms motion tick을 60fps frame 요청으로 승격하면 지난 motion deadline은 소비되어
    // 남은 frame limiter 간격 동안 zero-timeout busy loop를 만들지 않습니다.
    #[test]
    fn due_motion_waits_for_the_remaining_60_fps_frame_interval() {
        let started = Instant::now();
        let motion_tick = started + Duration::from_millis(16);
        let mut frames = FrameScheduler::new(FrameRateLimit::Fps60);
        frames.request(FrameRequest::Immediate);
        frames.rendered(started);
        let mut motion_deadline = Some(motion_tick);

        request_due_motion(&mut frames, true, &mut motion_deadline, motion_tick);
        let timeout = wait_timeout_at(
            None,
            motion_deadline,
            frames.deadline(motion_tick),
            motion_tick,
        );

        assert!(timeout.is_some_and(|timeout| timeout > Duration::ZERO));
        assert!(timeout.is_some_and(|timeout| timeout < Duration::from_millis(1)));
    }

    // 늦게 깨어난 frame은 놓친 tick을 재생하지 않고 epoch 기준 다음 경계 하나만 예약한다.
    #[test]
    fn late_frame_skips_missed_ticks_and_targets_the_next_epoch_boundary() {
        let epoch = Instant::now();
        let deadline = next_motion_deadline(
            epoch,
            Duration::from_millis(370),
            Some(Duration::from_millis(120)),
        )
        .unwrap();

        assert_eq!(deadline.duration_since(epoch), Duration::from_millis(480));
    }

    // 정확한 tick 경계에서 그린 frame도 같은 경계를 다시 요구하지 않고 다음 tick을 예약한다.
    #[test]
    fn exact_tick_boundary_schedules_the_following_tick() {
        let epoch = Instant::now();
        let deadline = next_motion_deadline(
            epoch,
            Duration::from_millis(120),
            Some(Duration::from_millis(120)),
        )
        .unwrap();

        assert_eq!(deadline.duration_since(epoch), Duration::from_millis(240));
    }

    // frame이 motion을 요구하지 않으면 runner는 별도의 시간 기반 wakeup을 만들지 않는다.
    #[test]
    fn absent_motion_demand_disarms_the_deadline() {
        assert_eq!(
            next_motion_deadline(Instant::now(), Duration::from_secs(1), None),
            None
        );
    }
}
