use std::{
    fs::{File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
};

use serde::Serialize;
use yo_core::{
    SessionId,
    session_repository::{
        LocalSessionReader, StoredRequestTraceRecord, StoredSessionReader, read_stored_session,
    },
};

#[derive(Clone, Debug)]
pub(super) struct AttemptInput {
    pub(super) request_id: String,
    pub(super) output_directory: PathBuf,
    pub(super) outcome_status: Option<String>,
    pub(super) outcome_durable_requests: Option<u64>,
    pub(super) has_receipt: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum State {
    Prepared,
    Claimed,
    ProcessStarted,
    DurableRequestObserved,
    Completed,
    FailedBeforeEffect,
    FailedOrUnknownEffect,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct Projection {
    pub(super) state: State,
    pub(super) attempt_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) request_id: Option<String>,
    pub(super) durable_request_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) blocking_reason: Option<String>,
    pub(super) next_action: &'static str,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SessionObservation {
    exists: bool,
    writer_active: bool,
    durable_requests: u64,
}

pub(super) fn prepared() -> Projection {
    projection(State::Prepared, 0, None, 0, None)
}

pub(super) fn project(mut attempts: Vec<AttemptInput>) -> Result<Projection, String> {
    if attempts.is_empty() {
        return Ok(prepared());
    }
    attempts.sort_by(|left, right| left.request_id.cmp(&right.request_id));
    let mut classified = attempts
        .iter()
        .map(classify_attempt)
        .collect::<Result<Vec<_>, _>>()?;
    let mut selected = if let Some(completed) = classified
        .iter()
        .find(|item| item.state == State::Completed)
        .cloned()
    {
        completed
    } else {
        classified.sort_by_key(|item| item.state);
        classified
            .pop()
            .expect("non-empty attempts produce classified attempts")
    };
    selected.attempt_count = attempts.len();
    if attempts.len() > 1 && selected.state != State::Completed {
        selected.state = State::FailedOrUnknownEffect;
        selected.blocking_reason = Some(
            "multiple claimed attempts exist for the current candidate; inspect receipts and Session evidence without sending another request"
                .to_owned(),
        );
        selected.next_action = "reconcile_unknown_delivery";
    }
    Ok(selected)
}

fn classify_attempt(attempt: &AttemptInput) -> Result<Projection, String> {
    if attempt.outcome_status.as_deref() == Some("completed") && attempt.has_receipt {
        return Ok(projection(
            State::Completed,
            1,
            Some(attempt.request_id.clone()),
            attempt.outcome_durable_requests.unwrap_or(1),
            None,
        ));
    }
    if let Some(status) = attempt.outcome_status.as_deref() {
        let durable = attempt.outcome_durable_requests.unwrap_or(0);
        let (state, reason) = if status == "failed" && durable == 0 {
            (
                State::FailedBeforeEffect,
                "the immutable attempt ended with no durable external request; inspect the outcome before preparing any separately authorized replacement",
            )
        } else {
            (
                State::FailedOrUnknownEffect,
                "the immutable attempt ended after a durable request or with inconsistent terminal artifacts; do not retry",
            )
        };
        return Ok(projection(
            state,
            1,
            Some(attempt.request_id.clone()),
            durable,
            Some(reason.to_owned()),
        ));
    }

    let observation = observe_sessions(&attempt.output_directory.join("sessions"))?;
    let capture_exists = [".review.stdout.tmp", ".review.stderr.tmp"]
        .iter()
        .any(|name| attempt.output_directory.join(name).exists());
    let (state, reason) = if observation.durable_requests > 0 && observation.writer_active {
        (
            State::DurableRequestObserved,
            Some(
                "the exact external request is durable and its owning Session writer is still active",
            ),
        )
    } else if observation.durable_requests > 0 {
        (
            State::FailedOrUnknownEffect,
            Some(
                "a durable external request exists but no terminal outcome or active Session writer is observable; do not retry",
            ),
        )
    } else if observation.writer_active || capture_exists {
        (
            State::ProcessStarted,
            Some(
                "the claimed delivery has started but has no terminal outcome; await or inspect this attempt without retrying",
            ),
        )
    } else if observation.exists {
        (
            State::FailedBeforeEffect,
            Some(
                "the claimed delivery Session is inactive and contains no durable external request",
            ),
        )
    } else {
        (
            State::Claimed,
            Some("the immutable delivery is claimed; another delivery invocation is forbidden"),
        )
    };
    Ok(projection(
        state,
        1,
        Some(attempt.request_id.clone()),
        observation.durable_requests,
        reason.map(str::to_owned),
    ))
}

fn projection(
    state: State,
    attempt_count: usize,
    request_id: Option<String>,
    durable_request_count: u64,
    blocking_reason: Option<String>,
) -> Projection {
    let next_action = match state {
        State::Prepared => "deliver_current_review",
        State::Claimed | State::ProcessStarted | State::DurableRequestObserved => {
            "await_current_delivery"
        },
        State::Completed => "interpret_review",
        State::FailedBeforeEffect => "reconcile_failed_delivery",
        State::FailedOrUnknownEffect => "reconcile_unknown_delivery",
    };
    Projection {
        state,
        attempt_count,
        request_id,
        durable_request_count,
        blocking_reason,
        next_action,
    }
}

fn observe_sessions(root: &Path) -> Result<SessionObservation, String> {
    if !root.exists() {
        return Ok(SessionObservation::default());
    }
    let reader = LocalSessionReader::open(root)
        .map_err(|error| format!("cannot inspect delivery Session repository: {error}"))?;
    let sessions = reader
        .discover()
        .map_err(|error| format!("cannot discover delivery Sessions: {error}"))?;
    if sessions.is_empty() {
        return Ok(SessionObservation {
            exists: true,
            ..SessionObservation::default()
        });
    }
    if sessions.len() != 1 {
        return Err(format!(
            "delivery attempt contains {} Sessions instead of exactly one",
            sessions.len()
        ));
    }
    let session_id = sessions[0].session_id();
    let history = read_stored_session(&reader, session_id)
        .map_err(|error| format!("cannot recover delivery Session: {error}"))?;
    let durable_requests = history
        .request_trace()
        .iter()
        .filter(|entry| {
            matches!(
                entry.record(),
                StoredRequestTraceRecord::RequestAccepted { .. }
            )
        })
        .count() as u64;
    Ok(SessionObservation {
        exists: true,
        writer_active: writer_is_active(root, session_id)?,
        durable_requests,
    })
}

fn writer_is_active(root: &Path, session_id: SessionId) -> Result<bool, String> {
    let path = root.join(format!("{session_id}.writer.lock"));
    let file = match OpenOptions::new().read(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "cannot inspect delivery Session writer lock {}: {error}",
                path.display()
            ));
        },
    };
    match File::try_lock_shared(&file) {
        Ok(()) => {
            let _ = File::unlock(&file);
            Ok(false)
        },
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(TryLockError::Error(error)) => Err(format!(
            "cannot inspect delivery Session writer lock {}: {error}",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(status: Option<&str>, durable: Option<u64>, receipt: bool) -> AttemptInput {
        AttemptInput {
            request_id: "sha256:request".to_owned(),
            output_directory: PathBuf::from("/definitely/missing/delivery"),
            outcome_status: status.map(str::to_owned),
            outcome_durable_requests: durable,
            has_receipt: receipt,
        }
    }

    // 완료 outcome과 receipt가 함께 있어야만 검토 결과 해석으로 넘어가며, claim만
    // 존재하는 상태는 다시 delivery하라는 권한으로 절대 축약하지 않습니다.
    #[test]
    fn terminal_artifacts_distinguish_completed_and_claimed_attempts() {
        let completed = project(vec![attempt(Some("completed"), Some(1), true)]).unwrap();
        assert_eq!(completed.state, State::Completed);
        assert_eq!(completed.next_action, "interpret_review");

        let claimed = project(vec![attempt(None, None, false)]).unwrap();
        assert_eq!(claimed.state, State::Claimed);
        assert_eq!(claimed.next_action, "await_current_delivery");
    }

    // 실패 outcome의 durable request 수가 0일 때와 1 이상일 때는 외부 효과의
    // 불확실성이 다르므로 별도 상태로 보존하고 어느 쪽도 자동 재시도를 제안하지 않습니다.
    #[test]
    fn failed_attempts_preserve_effect_boundary() {
        let before = project(vec![attempt(Some("failed"), Some(0), false)]).unwrap();
        assert_eq!(before.state, State::FailedBeforeEffect);
        assert_eq!(before.next_action, "reconcile_failed_delivery");

        let unknown = project(vec![attempt(Some("failed"), Some(1), false)]).unwrap();
        assert_eq!(unknown.state, State::FailedOrUnknownEffect);
        assert_eq!(unknown.next_action, "reconcile_unknown_delivery");
    }
}
