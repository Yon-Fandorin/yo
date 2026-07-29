use std::{
    io::{self, Write},
    panic,
};

use super::{LiveBackendError, LoopError, LoopExit};
use crate::{
    runner::{RunError, RunOutcome, TerminalOutcome},
    terminal::{
        backend::unix::UnixMode,
        mode::{
            CleanupFailure, CleanupFailures, SessionFailure, SessionFailureCause,
            panic_route::{PanicDiagnostic, PanicOutcome, PanicPayload, PanicRouteError},
            screen::InlineCloseReport,
        },
    },
};

pub(super) struct LiveRunReport {
    pub(super) operation: Result<Result<LoopExit, LoopError>, PanicPayload>,
    pub(super) cleanup: LiveCleanup,
    pub(super) output: Option<String>,
}

pub(super) enum LiveCleanup {
    Inline(InlineCloseReport<UnixMode, LiveBackendError>),
    Fullscreen(Result<(), CleanupFailures<UnixMode, LiveBackendError>>),
}

type RunnerBoundary = Result<PanicOutcome<Result<LiveRunReport, RunError>>, PanicRouteError>;

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

pub(super) fn finish(outcome: RunnerBoundary) -> Result<TerminalOutcome, RunError> {
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
    finish_report(report, outcome.diagnostic)
}

fn finish_report(
    report: LiveRunReport,
    diagnostic: Option<PanicDiagnostic>,
) -> Result<TerminalOutcome, RunError> {
    let output = report.output;
    let operation = match report.operation {
        Ok(operation) => operation,
        Err(payload) => {
            emit_panic_cleanup(&report.cleanup);
            resume_after_cleanup_panic(payload, diagnostic)
        },
    };

    match operation {
        Ok(LoopExit::User) => {
            require_clean_close(report.cleanup)?;
            Ok(TerminalOutcome::Exited(RunOutcome::user_requested(output)))
        },
        Ok(LoopExit::Termination) => {
            require_clean_close(report.cleanup)?;
            Ok(TerminalOutcome::Exited(RunOutcome::termination_requested(
                output,
            )))
        },
        Ok(LoopExit::Suspend) => {
            require_clean_close(report.cleanup)?;
            Ok(TerminalOutcome::SuspendRequested)
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

fn require_clean_close(report: LiveCleanup) -> Result<(), RunError> {
    let detail = cleanup_detail(&report);
    if detail.is_empty() {
        Ok(())
    } else {
        Err(RunError::new("restoring terminal state failed", detail))
    }
}

fn cleanup_detail(report: &LiveCleanup) -> String {
    let mut failures = Vec::new();
    match report {
        LiveCleanup::Inline(report) => {
            if let Err(error) = &report.viewport {
                failures.push(format!("inline viewport: {error:?}"));
            }
            if let Err(error) = &report.terminal {
                failures.push(format!("terminal: {error:?}"));
            }
        },
        LiveCleanup::Fullscreen(report) => {
            if let Err(error) = report {
                failures.push(format!("terminal: {error:?}"));
            }
        },
    }
    failures.join("; ")
}

fn emit_panic_cleanup(report: &LiveCleanup) {
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
    use std::{
        io,
        panic::{AssertUnwindSafe, catch_unwind},
    };

    use super::{
        LiveBackendError, LiveCleanup, LiveRunReport, LoopError, LoopExit, UnixMode,
        cleanup_detail, entry_failure, finish_report,
    };
    use crate::{
        runner::{ExitReason, TerminalOutcome},
        terminal::{
            backend::unix::UnixBackendError,
            mode::{
                CleanupFailure, CleanupFailureCause, CleanupFailures, CleanupStep, SessionFailure,
                SessionFailureCause,
                panic_route::{PANIC_ROUTE_TEST_OWNER, catch_owner_panic},
                screen::InlineCloseReport,
            },
        },
    };

    fn failed_fullscreen_cleanup(messages: &[&str]) -> LiveCleanup {
        LiveCleanup::Fullscreen(Err(CleanupFailures {
            failures: messages
                .iter()
                .enumerate()
                .map(|(index, message)| CleanupFailure {
                    step: if index == 0 {
                        CleanupStep::Mode(UnixMode::AlternateScreen)
                    } else {
                        CleanupStep::TtyState
                    },
                    cause: CleanupFailureCause::Error(UnixBackendError::Output(io::Error::other(
                        *message,
                    ))),
                })
                .collect(),
        }))
    }

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

    // 실행 오류와 여러 cleanup 오류가 겹치면 실행 오류를 주원인으로 유지하면서 모든 복구 실패를
    // 덧붙인다.
    #[test]
    fn operation_error_retains_primary_and_every_cleanup_failure() {
        let cleanup = failed_fullscreen_cleanup(&["leaving screen", "restoring termios"]);
        let report = LiveRunReport {
            operation: Ok(Err(LoopError::Agent("agent disconnected".to_owned()))),
            cleanup,
            output: None,
        };

        let error = finish_report(report, None).unwrap_err().to_string();

        assert!(error.contains("agent disconnected"));
        assert!(error.contains("leaving screen"));
        assert!(error.contains("restoring termios"));
    }

    // 종료 신호를 관찰했더라도 terminal 복구가 실패하면 정상 종료로 숨기지 않고 복구 오류를
    // 반환한다.
    #[test]
    fn termination_exit_reports_cleanup_failure_before_host_replays_signal() {
        let report = LiveRunReport {
            operation: Ok(Ok(LoopExit::Termination)),
            cleanup: failed_fullscreen_cleanup(&["restoring termios"]),
            output: None,
        };

        let error = finish_report(report, None).unwrap_err().to_string();

        assert!(error.contains("restoring terminal state failed"));
        assert!(error.contains("restoring termios"));
    }

    // panic과 여러 cleanup 오류가 겹쳐도 모든 복구 실패를 표현할 수 있고 원래 panic payload를
    // 그대로 재전파한다.
    #[test]
    fn panic_retains_payload_after_collecting_every_cleanup_failure() {
        let cleanup = failed_fullscreen_cleanup(&["leaving screen", "restoring termios"]);
        let detail = cleanup_detail(&cleanup);
        assert!(detail.contains("leaving screen"));
        assert!(detail.contains("restoring termios"));
        let payload = catch_unwind(AssertUnwindSafe(|| panic!("runner panic"))).unwrap_err();
        let report = LiveRunReport {
            operation: Err(payload),
            cleanup,
            output: None,
        };

        let resumed = catch_unwind(AssertUnwindSafe(|| {
            let _ = finish_report(report, None);
        }))
        .unwrap_err();

        assert_eq!(resumed.downcast_ref::<&str>(), Some(&"runner panic"));
    }

    // cleanup이 성공한 termination 경로는 host가 같은 signal을 재생할 수 있도록 정상 사유를
    // 보존한다.
    #[test]
    fn clean_termination_preserves_the_termination_exit_reason() {
        let report = LiveRunReport {
            operation: Ok(Ok(LoopExit::Termination)),
            cleanup: LiveCleanup::Inline(InlineCloseReport {
                viewport: Ok(crate::terminal::mode::inline::InlineRestoreOutcome::Cleared),
                terminal: Ok(()),
            }),
            output: Some("retained\n".to_owned()),
        };

        let TerminalOutcome::Exited(outcome) = finish_report(report, None).unwrap() else {
            panic!("termination must complete the application session");
        };

        assert_eq!(outcome.reason(), ExitReason::TerminationRequested);
        assert_eq!(outcome.output(), Some("retained\n"));
    }

    // Fullscreen 정상 종료는 main screen 복원 뒤 별도 session 출력을 만들지 않는다.
    #[test]
    fn clean_fullscreen_exit_has_no_session_output() {
        let report = LiveRunReport {
            operation: Ok(Ok(LoopExit::User)),
            cleanup: LiveCleanup::Fullscreen(Ok(())),
            output: None,
        };

        let TerminalOutcome::Exited(outcome) = finish_report(report, None).unwrap() else {
            panic!("user exit must complete the application session");
        };

        assert_eq!(outcome.reason(), ExitReason::UserRequested);
        assert_eq!(outcome.output(), None);
    }

    // 일시정지 요청은 terminal 복구가 모두 성공한 뒤에만 host로 전달되며 종료용 session
    // 출력도 만들지 않는다.
    #[test]
    fn clean_suspend_returns_only_after_terminal_restoration() {
        let report = LiveRunReport {
            operation: Ok(Ok(LoopExit::Suspend)),
            cleanup: LiveCleanup::Fullscreen(Ok(())),
            output: None,
        };

        assert_eq!(
            finish_report(report, None).unwrap(),
            TerminalOutcome::SuspendRequested
        );
    }

    // 일시정지 요청 중 terminal 복구가 실패하면 host가 프로세스를 멈추지 않도록 구조화된
    // 복구 오류를 반환한다.
    #[test]
    fn suspend_cleanup_failure_prevents_host_suspension() {
        let report = LiveRunReport {
            operation: Ok(Ok(LoopExit::Suspend)),
            cleanup: failed_fullscreen_cleanup(&["restoring termios"]),
            output: None,
        };

        let error = finish_report(report, None).unwrap_err().to_string();

        assert!(error.contains("restoring terminal state failed"));
        assert!(error.contains("restoring termios"));
    }
}
