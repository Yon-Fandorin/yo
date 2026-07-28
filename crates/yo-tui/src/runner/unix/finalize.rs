use std::{
    io::{self, Write},
    panic,
};

use super::{LiveBackendError, LiveInlineReport, LoopExit};
use crate::{
    runner::{RunError, RunOutcome},
    terminal::{
        backend::unix::UnixMode,
        mode::{
            CleanupFailure, SessionFailure, SessionFailureCause,
            panic_route::{PanicDiagnostic, PanicOutcome, PanicPayload, PanicRouteError},
            screen::InlineCloseReport,
        },
    },
};

type RunnerBoundary = Result<PanicOutcome<Result<LiveInlineReport, RunError>>, PanicRouteError>;

pub(super) fn entry_failure(
    failure: SessionFailure<SessionFailureCause<LiveBackendError>, UnixMode, LiveBackendError>,
) -> RunError {
    let SessionFailure { primary, cleanup } = failure;
    match primary {
        SessionFailureCause::Error(error) => RunError::new(
            "entering terminal mode failed",
            format!("primary: {error:?}; cleanup: {cleanup:?}"),
        ),
        SessionFailureCause::Panicked(payload) => {
            emit_entry_panic_cleanup(&cleanup);
            panic::resume_unwind(payload)
        },
    }
}

pub(super) fn finish(outcome: RunnerBoundary) -> Result<RunOutcome, RunError> {
    let outcome = outcome.map_err(|error| {
        RunError::new(
            "installing terminal panic route failed",
            format!("{error:?}"),
        )
    })?;
    let report = match outcome.result {
        Ok(Ok(report)) => report,
        Ok(Err(error)) => return Err(error),
        Err(payload) => resume_after_cleanup_panic(payload, outcome.diagnostic),
    };
    let operation = match report.operation {
        Ok(operation) => operation,
        Err(payload) => {
            emit_panic_cleanup(&report.cleanup);
            resume_after_cleanup_panic(payload, outcome.diagnostic)
        },
    };

    match operation {
        Ok(LoopExit::UserRequested) => {
            require_clean_close(report.cleanup)?;
            Ok(RunOutcome::user_requested())
        },
        Ok(LoopExit::TerminationRequested) => {
            require_clean_close(report.cleanup)?;
            Ok(RunOutcome::termination_requested())
        },
        Err(error) => {
            let cleanup = cleanup_detail(&report.cleanup);
            Err(RunError::with_source(
                "running the terminal UI failed",
                if cleanup.is_empty() {
                    error.detail()
                } else {
                    format!("{}; {cleanup}", error.detail())
                },
                error,
            ))
        },
    }
}

fn emit_entry_panic_cleanup<M, E>(failures: &[CleanupFailure<M, E>])
where
    M: std::fmt::Debug,
    E: std::fmt::Debug,
{
    if !failures.is_empty() {
        let _ = writeln!(
            io::stderr().lock(),
            "terminal cleanup after entry panic failed: {failures:?}"
        );
    }
}

fn require_clean_close<M, E>(report: InlineCloseReport<M, E>) -> Result<(), RunError>
where
    M: std::fmt::Debug,
    E: std::fmt::Debug,
{
    let detail = cleanup_detail(&report);
    if detail.is_empty() {
        Ok(())
    } else {
        Err(RunError::new("restoring terminal state failed", detail))
    }
}

fn cleanup_detail<M, E>(report: &InlineCloseReport<M, E>) -> String
where
    M: std::fmt::Debug,
    E: std::fmt::Debug,
{
    let mut failures = Vec::new();
    if let Err(error) = &report.viewport {
        failures.push(format!("inline viewport: {error:?}"));
    }
    if let Err(error) = &report.terminal {
        failures.push(format!("terminal: {error:?}"));
    }
    failures.join("; ")
}

fn emit_panic_cleanup<M, E>(report: &InlineCloseReport<M, E>)
where
    M: std::fmt::Debug,
    E: std::fmt::Debug,
{
    let detail = cleanup_detail(report);
    if !detail.is_empty() {
        let _ = writeln!(
            io::stderr().lock(),
            "terminal cleanup after panic failed: {detail}"
        );
    }
}

fn resume_after_cleanup_panic(payload: PanicPayload, diagnostic: Option<PanicDiagnostic>) -> ! {
    if let Some(diagnostic) = diagnostic {
        let _ = diagnostic.emit(&mut io::stderr().lock());
    }
    panic::resume_unwind(payload)
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::{LiveBackendError, UnixMode, entry_failure};
    use crate::terminal::mode::{
        SessionFailure, SessionFailureCause,
        panic_route::{PANIC_ROUTE_TEST_OWNER, catch_owner_panic},
    };

    // 진입 panic은 복구 뒤 일반 오류로 바뀌지 않고 원래 payload와 진단으로 다시 unwind한다.
    #[test]
    fn entry_panic_is_resumed_through_the_outer_route() {
        let _route_test = PANIC_ROUTE_TEST_OWNER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let routed = catch_owner_panic(AssertUnwindSafe(|| {
            let payload = catch_unwind(AssertUnwindSafe(|| panic!("entry panic"))).unwrap_err();
            let failure = SessionFailure::<
                SessionFailureCause<LiveBackendError>,
                UnixMode,
                LiveBackendError,
            > {
                primary: SessionFailureCause::Panicked(payload),
                cleanup: Vec::new(),
            };
            let _ = entry_failure(failure);
        }))
        .unwrap();
        let payload = routed
            .result
            .expect_err("entry panic must cross the complete runner boundary");

        assert_eq!(payload.downcast_ref::<&str>(), Some(&"entry panic"));
        assert_eq!(routed.diagnostic.unwrap().message, "entry panic");
    }
}
