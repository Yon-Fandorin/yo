use std::time::{Duration, Instant};

/// Maximum live TUI presentation rate.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum FrameRateLimit {
    /// Coalesces frames at roughly 8.33 ms intervals.
    #[default]
    Fps120,
    /// Coalesces frames at roughly 16.67 ms intervals.
    Fps60,
}

impl FrameRateLimit {
    pub(super) const fn interval(self) -> Duration {
        let frames_per_second = match self {
            Self::Fps120 => 120,
            Self::Fps60 => 60,
        };
        Duration::from_nanos(1_000_000_000 / frames_per_second)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FrameRequest {
    Immediate,
    Coalesced,
}

pub(super) struct FrameScheduler {
    interval: Duration,
    last_frame: Option<Instant>,
    requested: bool,
    immediate: bool,
    marker_deadline: Option<Instant>,
    render_cost: Duration,
}

impl FrameScheduler {
    pub(super) fn new(limit: FrameRateLimit) -> Self {
        Self {
            interval: limit.interval(),
            last_frame: None,
            requested: false,
            immediate: false,
            marker_deadline: None,
            render_cost: Duration::ZERO,
        }
    }

    pub(super) fn request(&mut self, request: FrameRequest) {
        self.requested = true;
        self.immediate |= request == FrameRequest::Immediate;
    }

    pub(super) fn deadline(&self, now: Instant) -> Option<Instant> {
        if !self.requested {
            return None;
        }
        if self.immediate {
            return Some(now);
        }
        let ordinary = self
            .last_frame
            .and_then(|last| last.checked_add(self.interval))
            .map_or(now, |deadline| deadline.max(now));
        // Do not spend the next frame slot on a shimmer/input redraw just
        // before a marker transition. Preserve the FPS bound, merging those
        // redraws into the marker boundary instead of delaying the marker.
        if let Some(marker) = self.marker_deadline
            && ordinary < marker
            && marker.duration_since(ordinary) < self.interval.saturating_add(self.render_cost)
        {
            return Some(marker);
        }
        Some(ordinary)
    }

    pub(super) fn is_due(&self, now: Instant) -> bool {
        self.deadline(now).is_some_and(|deadline| deadline <= now)
    }

    pub(super) fn suppress_pending(&mut self) {
        self.requested = false;
        self.immediate = false;
        self.marker_deadline = None;
    }

    pub(super) fn rendered(&mut self, now: Instant) {
        self.last_frame = Some(now);
        self.requested = false;
        self.immediate = false;
    }

    pub(super) fn reserve_marker(&mut self, deadline: Option<Instant>) {
        self.marker_deadline = deadline;
    }

    pub(super) fn rendered_with_cost(&mut self, started: Instant, completed: Instant) {
        self.render_cost = completed.saturating_duration_since(started);
        self.rendered(completed);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{FrameRateLimit, FrameRequest, FrameScheduler};

    // 기본 frame 제한은 120fps이며 한 frame 직후 요청을 약 8.33ms 경계까지 합칩니다.
    #[test]
    fn default_limit_coalesces_at_120_fps() {
        let started = Instant::now();
        let mut scheduler = FrameScheduler::new(FrameRateLimit::default());
        scheduler.request(FrameRequest::Immediate);
        scheduler.rendered(started);
        scheduler.request(FrameRequest::Coalesced);

        assert_eq!(
            scheduler.deadline(started),
            started.checked_add(Duration::from_nanos(8_333_333))
        );
    }

    // 60fps 선택은 동일한 합치기 계약을 유지하면서 frame 간격만 약 16.67ms로 낮춥니다.
    #[test]
    fn optional_limit_coalesces_at_60_fps() {
        let started = Instant::now();
        let mut scheduler = FrameScheduler::new(FrameRateLimit::Fps60);
        scheduler.request(FrameRequest::Immediate);
        scheduler.rendered(started);
        scheduler.request(FrameRequest::Coalesced);

        assert_eq!(
            scheduler.deadline(started),
            started.checked_add(Duration::from_nanos(16_666_666))
        );
    }

    // 즉시 요청은 대기 중인 coalesced frame을 현재 시점으로 승격합니다.
    #[test]
    fn immediate_request_preempts_coalescing_deadline() {
        let started = Instant::now();
        let now = started + Duration::from_millis(1);
        let mut scheduler = FrameScheduler::new(FrameRateLimit::Fps120);
        scheduler.request(FrameRequest::Immediate);
        scheduler.rendered(started);
        scheduler.request(FrameRequest::Coalesced);
        scheduler.request(FrameRequest::Immediate);

        assert_eq!(scheduler.deadline(now), Some(now));
    }

    // 지난 marker 예약은 FPS 제한을 우회하거나 과거 deadline으로 busy loop를 만들지 않는다.
    #[test]
    fn overdue_marker_preserves_frame_limit() {
        let now = Instant::now();
        let mut scheduler = FrameScheduler::new(FrameRateLimit::Fps60);
        scheduler.rendered(now);
        scheduler.reserve_marker(Some(now));
        scheduler.request(FrameRequest::Coalesced);
        assert_eq!(
            scheduler.deadline(now),
            Some(now + FrameRateLimit::Fps60.interval())
        );
        assert!(scheduler.is_due(now + Duration::from_millis(20)));
    }

    // motion이 끝나 예약을 해제하면 가까운 marker 경계 때문에 일반 입력 frame이 지연되지 않는다.
    #[test]
    fn clearing_marker_releases_reserved_frame_slot() {
        let now = Instant::now();
        let mut scheduler = FrameScheduler::new(FrameRateLimit::Fps120);
        scheduler.reserve_marker(Some(now + Duration::from_millis(4)));
        assert_eq!(scheduler.deadline(now), None);
        scheduler.request(FrameRequest::Coalesced);
        assert_eq!(
            scheduler.deadline(now),
            Some(now + Duration::from_millis(4))
        );
        scheduler.reserve_marker(None);
        assert_eq!(scheduler.deadline(now), Some(now));
    }

    // 보이지 않는 geometry에서 immediate와 coalesced 요청을 버려도 마지막으로 완료한
    // frame 시각은 유지되며, 이후 ordinary 요청과 즉시 요청은 각자의 원래 경계로 재개됩니다.
    #[test]
    fn suppression_retires_pending_requests_without_advancing_the_last_frame() {
        let completed = Instant::now();
        let now = completed + Duration::from_millis(1);
        let mut scheduler = FrameScheduler::new(FrameRateLimit::Fps60);
        scheduler.request(FrameRequest::Immediate);
        scheduler.rendered(completed);

        scheduler.request(FrameRequest::Immediate);
        scheduler.suppress_pending();
        assert_eq!(scheduler.deadline(now), None);

        scheduler.request(FrameRequest::Coalesced);
        assert_eq!(
            scheduler.deadline(now),
            completed.checked_add(Duration::from_nanos(16_666_666))
        );
        scheduler.suppress_pending();
        assert_eq!(scheduler.deadline(now), None);

        scheduler.request(FrameRequest::Immediate);
        assert!(scheduler.is_due(now));
        scheduler.rendered(now);
        assert_eq!(scheduler.deadline(now), None);
    }
}
