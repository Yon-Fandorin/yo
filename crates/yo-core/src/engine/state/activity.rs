use super::{ActivityState, EngineState, RequestState};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityUpdate, AgentEvent,
    Failure, RequestId, TurnOutcome, TurnRef,
    engine::{AgentRejection, ExpectedResponse, ResponseKind},
};

impl EngineState {
    pub(in crate::engine) fn fail_active_turn(&mut self, failure: Failure) -> Vec<AgentEvent> {
        self.finish_active_turn(
            ActivityOutcome::Failed(failure.clone()),
            TurnOutcome::Failed(failure),
        )
    }

    pub(in crate::engine) fn interrupt_active_turn(&mut self) -> Vec<AgentEvent> {
        self.finish_active_turn(ActivityOutcome::Interrupted, TurnOutcome::Interrupted)
    }

    fn finish_active_turn(
        &mut self,
        activity_outcome: ActivityOutcome,
        turn_outcome: TurnOutcome,
    ) -> Vec<AgentEvent> {
        let Some(turn_state) = self
            .session
            .as_mut()
            .and_then(|session| session.active_turn_mut())
        else {
            return Vec::new();
        };

        let mut events = Vec::new();
        for activity in &turn_state.activity_order {
            let state = turn_state
                .activities
                .get_mut(activity)
                .expect("activity order and storage stay synchronized");
            if state.active {
                state.active = false;
                events.push(AgentEvent::ActivityFinished {
                    activity: *activity,
                    outcome: activity_outcome.clone(),
                });
            }
        }
        let turn = turn_state.turn;
        turn_state.outcome = Some(turn_outcome.clone());
        events.push(AgentEvent::TurnFinished {
            turn,
            outcome: turn_outcome,
        });
        events
    }

    pub(in crate::engine) fn start_activity(
        &mut self,
        activity: ActivityRef,
        kind: ActivityKind,
    ) -> Result<AgentEvent, AgentRejection> {
        let turn = self.require_active_turn_mut(activity.turn())?;
        if turn.activities.contains_key(&activity) {
            return Err(AgentRejection::DuplicateActivity { activity });
        }

        if let Some((request_id, expected)) = expected_response(kind) {
            let request = ActivityRequestRef::new(activity, request_id);
            if turn
                .requests
                .keys()
                .any(|candidate| candidate.request_id() == request_id)
            {
                return Err(AgentRejection::DuplicateRequest { request });
            }
            turn.requests.insert(
                request,
                RequestState {
                    expected,
                    answered: false,
                    response_recorded: false,
                },
            );
            turn.request_order.push(request);
        }
        if let Some((request_id, actual)) = recorded_response(kind) {
            let (request, request_state) = turn
                .requests
                .iter_mut()
                .find(|(request, _)| request.request_id() == request_id)
                .ok_or(AgentRejection::ResponseRequestNotFound {
                    turn: activity.turn(),
                    request_id,
                })?;
            if request_state.expected != actual.into() {
                return Err(AgentRejection::ResponseKindMismatch {
                    request: *request,
                    expected: request_state.expected,
                    actual,
                });
            }
            if !request_state.answered {
                return Err(AgentRejection::ResponseNotAnswered { request: *request });
            }
            if request_state.response_recorded {
                return Err(AgentRejection::ResponseAlreadyRecorded { request: *request });
            }
            request_state.response_recorded = true;
        }

        turn.activities
            .insert(activity, ActivityState { active: true });
        turn.activity_order.push(activity);
        Ok(AgentEvent::ActivityStarted { activity, kind })
    }

    pub(in crate::engine) fn update_activity(
        &mut self,
        activity: ActivityRef,
        update: ActivityUpdate,
    ) -> Result<AgentEvent, AgentRejection> {
        let activity_state = self.require_active_activity_mut(activity)?;
        if !activity_state.active {
            return Err(AgentRejection::ActivityNotActive { activity });
        }
        Ok(AgentEvent::ActivityUpdated { activity, update })
    }

    pub(in crate::engine) fn finish_activity(
        &mut self,
        activity: ActivityRef,
        outcome: ActivityOutcome,
    ) -> Result<AgentEvent, AgentRejection> {
        let activity_state = self.require_active_activity_mut(activity)?;
        if !activity_state.active {
            return Err(AgentRejection::ActivityNotActive { activity });
        }
        activity_state.active = false;
        Ok(AgentEvent::ActivityFinished { activity, outcome })
    }

    pub(in crate::engine) fn finish_turn(
        &mut self,
        turn: TurnRef,
        outcome: TurnOutcome,
    ) -> Result<AgentEvent, AgentRejection> {
        let turn_state = self.require_active_turn_mut(turn)?;
        if let Some(activity) = turn_state
            .activity_order
            .iter()
            .copied()
            .find(|activity| turn_state.activities[activity].active)
        {
            return Err(AgentRejection::ActivityStillActive { activity });
        }
        if outcome == TurnOutcome::Completed {
            for request in &turn_state.request_order {
                let state = &turn_state.requests[request];
                if !state.answered {
                    return Err(AgentRejection::RequestStillUnanswered { request: *request });
                }
                if !state.response_recorded {
                    return Err(AgentRejection::ResponseNotRecorded { request: *request });
                }
            }
        }
        turn_state.outcome = Some(outcome.clone());
        Ok(AgentEvent::TurnFinished { turn, outcome })
    }
}

fn expected_response(kind: ActivityKind) -> Option<(RequestId, ExpectedResponse)> {
    match kind {
        ActivityKind::ApprovalRequest { request_id } => {
            Some((request_id, ExpectedResponse::Approval))
        },
        ActivityKind::UserInputRequest { request_id } => {
            Some((request_id, ExpectedResponse::UserInput))
        },
        _ => None,
    }
}

fn recorded_response(kind: ActivityKind) -> Option<(RequestId, ResponseKind)> {
    match kind {
        ActivityKind::ApprovalResponse { request_id } => Some((request_id, ResponseKind::Approval)),
        ActivityKind::UserInputResponse { request_id } => {
            Some((request_id, ResponseKind::UserInput))
        },
        _ => None,
    }
}

impl From<ResponseKind> for ExpectedResponse {
    fn from(kind: ResponseKind) -> Self {
        match kind {
            ResponseKind::Approval => Self::Approval,
            ResponseKind::UserInput => Self::UserInput,
        }
    }
}
