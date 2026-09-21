use std::{
    collections::{HashSet, VecDeque},
    hash::Hash,
};

use yo_core::{ActivityRequestRef, TurnRef};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AttentionKind {
    Turn(TurnRef),
    Request(ActivityRequestRef),
}

#[derive(Debug, Default)]
pub(super) struct AttentionState {
    enabled: bool,
    armed: bool,
    historical_through: Option<TurnRef>,
    pending: Option<AttentionKind>,
    notified_turns: HashSet<TurnRef>,
    notified_turn_order: VecDeque<TurnRef>,
    notified_requests: HashSet<ActivityRequestRef>,
    notified_request_order: VecDeque<ActivityRequestRef>,
}

const DEDUPE_LIMIT: usize = 64;

impl AttentionState {
    pub(super) fn set_history_cutoff(&mut self, turn: Option<TurnRef>) {
        self.historical_through = turn;
    }

    pub(super) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.pending = None;
        }
    }

    pub(super) fn arm(&mut self) {
        if self.enabled {
            self.armed = true;
        }
    }

    pub(super) fn observe_turn_started(&mut self) {
        if matches!(self.pending, Some(AttentionKind::Turn(_))) {
            self.pending = None;
        }
    }

    pub(super) fn observe_turn_finished(&mut self, turn: TurnRef) {
        if self.enabled && self.armed && self.is_live(turn) && !self.notified_turns.contains(&turn)
        {
            self.pending = Some(AttentionKind::Turn(turn));
        }
    }

    pub(super) fn observe_request(&mut self, request: ActivityRequestRef) {
        if self.enabled
            && self.armed
            && self.is_live(request.activity().turn())
            && !self.notified_requests.contains(&request)
        {
            // A request is the actionable result of its turn, so it supersedes any
            // completion candidate waiting for the same live observation window.
            if let Some(AttentionKind::Turn(turn)) = self.pending
                && turn == request.activity().turn()
            {
                self.remember_turn(turn);
            }
            self.pending = Some(AttentionKind::Request(request));
        }
    }

    pub(super) fn observe_request_finished(&mut self, activity: yo_core::ActivityRef) {
        if matches!(self.pending, Some(AttentionKind::Request(request)) if request.activity() == activity)
        {
            self.pending = None;
        }
    }

    pub(super) fn take_request(&mut self) -> Option<ActivityRequestRef> {
        let Some(AttentionKind::Request(request)) = self.pending else {
            return None;
        };
        self.pending = None;
        self.remember_request(request);
        Some(request)
    }

    pub(super) fn take_turn(&mut self, turn: TurnRef) -> bool {
        if self.pending != Some(AttentionKind::Turn(turn)) {
            return false;
        }
        self.pending = None;
        self.remember_turn(turn);
        true
    }

    pub(super) fn pending(&self) -> Option<AttentionKind> {
        self.pending
    }

    fn is_live(&self, turn: TurnRef) -> bool {
        self.historical_through.is_none_or(|cutoff| {
            turn.session_id() != cutoff.session_id() || turn.turn_id() > cutoff.turn_id()
        })
    }

    fn remember_turn(&mut self, turn: TurnRef) {
        remember(
            &mut self.notified_turns,
            &mut self.notified_turn_order,
            turn,
        );
    }

    fn remember_request(&mut self, request: ActivityRequestRef) {
        remember(
            &mut self.notified_requests,
            &mut self.notified_request_order,
            request,
        );
    }
}

fn remember<T>(set: &mut HashSet<T>, order: &mut VecDeque<T>, value: T)
where
    T: Copy + Eq + Hash,
{
    if !set.insert(value) {
        return;
    }
    order.push_back(value);
    if order.len() > DEDUPE_LIMIT
        && let Some(expired) = order.pop_front()
    {
        set.remove(&expired);
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use yo_core::{
        ActivityId, ActivityRef, ActivityRequestRef, RequestId, SessionId, TurnId, TurnRef,
    };

    use super::AttentionState;

    fn session() -> SessionId {
        "01890f00-0000-7000-8000-000000000001"
            .parse()
            .expect("valid fixture UUID")
    }

    fn turn(value: u64) -> TurnRef {
        TurnRef::new(session(), TurnId::new(NonZeroU64::new(value).unwrap()))
    }

    fn request(value: u64) -> ActivityRequestRef {
        ActivityRequestRef::new(
            ActivityRef::new(turn(1), ActivityId::new(NonZeroU64::new(value).unwrap())),
            RequestId::new(NonZeroU64::new(value).unwrap()),
        )
    }

    // 알림 설정을 끄면 live 후보가 생겨도 terminal 출력 요청을 만들지 않습니다.
    #[test]
    fn disabled_state_ignores_live_candidates() {
        let mut attention = AttentionState::default();
        attention.observe_turn_finished(turn(1));
        assert_eq!(attention.pending(), None);
    }

    // 같은 Turn의 completion이 질문으로 대체되면 질문을 한 번만 소비합니다.
    #[test]
    fn a_request_supersedes_and_deduplicates_completion() {
        let mut attention = AttentionState::default();
        attention.set_enabled(true);
        attention.arm();
        let current = turn(1);
        attention.observe_turn_finished(current);
        let request = request(1);
        attention.observe_request(request);
        assert_eq!(attention.take_request(), Some(request));
        attention.observe_request(request);
        assert_eq!(attention.take_request(), None);
        attention.observe_turn_finished(current);
        assert!(!attention.take_turn(current));
    }

    // 자동 후속 Turn이 시작되면 중간 completion 후보를 폐기합니다.
    #[test]
    fn a_new_turn_cancels_an_intermediate_completion() {
        let mut attention = AttentionState::default();
        attention.set_enabled(true);
        attention.arm();
        let first = turn(1);
        attention.observe_turn_finished(first);
        attention.observe_turn_started();
        assert_eq!(attention.pending(), None);
    }

    // Activity와 request ID가 모두 다른 요청은 각각 독립적인 알림 대상입니다.
    #[test]
    fn request_identity_includes_activity_and_request_id() {
        let mut attention = AttentionState::default();
        attention.set_enabled(true);
        attention.arm();
        let first = request(1);
        let second = ActivityRequestRef::new(
            first.activity(),
            RequestId::new(NonZeroU64::new(2).unwrap()),
        );
        attention.observe_request(first);
        assert_eq!(attention.take_request(), Some(first));
        attention.observe_request(second);
        assert_eq!(attention.take_request(), Some(second));
        assert!(attention.pending().is_none());
    }
}
