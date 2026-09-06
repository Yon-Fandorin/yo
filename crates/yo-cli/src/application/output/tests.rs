use std::process::ExitCode;

use super::{
    finish_account_output, finish_command_output, write_cli_diagnostics_to,
    write_cli_diagnostics_to_and_flush,
};
use crate::{
    application::account_exit_code,
    command::{AccountCompletion, AccountRunOutput},
    diagnostic::{AppError, CliDiagnostic},
};

struct FlushFails;

impl std::io::Write for FlushFails {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::other("flush failed"))
    }
}

// 일부 refresh가 실패해도 이미 만든 account 출력은 먼저 publish해야 합니다.
#[test]
fn account_output_is_published_before_a_partial_refresh_error() {
    let mut published = None;
    let result = finish_account_output(
        AccountRunOutput {
            output: "partial account output\n".to_owned(),
            diagnostics: vec![CliDiagnostic::warning("one warning")],
            completion: AccountCompletion::RefreshFailures,
        },
        |output| {
            published = Some(output);
            Ok(())
        },
        |diagnostics| {
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0].message(), "one warning");
            Ok(())
        },
    );

    assert_eq!(published.as_deref(), Some("partial account output\n"));
    assert!(matches!(result, Ok(AccountCompletion::RefreshFailures)));
}

// JSON은 stdout에 정확히 한 문서만 남기고, warning은 그 뒤 stderr에 한 번만 게시합니다.
#[test]
fn account_json_routes_warning_after_unchanged_stdout() {
    let stdout = std::cell::RefCell::new(String::new());
    let stderr = std::cell::RefCell::new(Vec::new());
    let events = std::cell::RefCell::new(Vec::new());
    let output = "{\"schema\":\"yo.account-capacity/v1alpha3\",\"provider\":\"codex\",\"account\":\"person@example.test\",\"limits\":[],\"errors\":[{\"target\":\"Local Grok\",\"message\":\"login required\"}]}\n";

    assert!(matches!(
        finish_account_output(
            AccountRunOutput {
                output: output.to_owned(),
                diagnostics: vec![CliDiagnostic::warning("Codex compatibility warning")],
                completion: AccountCompletion::RefreshFailures,
            },
            |output| {
                events.borrow_mut().push("stdout");
                stdout.replace(output);
                Ok(())
            },
            |diagnostics| {
                events.borrow_mut().push("stderr");
                write_cli_diagnostics_to(diagnostics, &mut *stderr.borrow_mut())
            },
        ),
        Ok(AccountCompletion::RefreshFailures)
    ));

    let stdout = stdout.into_inner();
    let stderr = String::from_utf8(stderr.into_inner()).unwrap();
    let decoded: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(!stdout.contains("Codex compatibility warning"));
    assert_eq!(decoded["errors"][0]["target"], "Local Grok");
    assert_eq!(decoded["errors"][0]["message"], "login required");
    assert_eq!(stderr.matches("yo: warning:").count(), 1);
    assert!(!stderr.contains("yo: error:"));
    assert_eq!(events.into_inner(), vec!["stdout", "stderr"]);
}

// Account completion은 예상 refresh failure를 stderr error로 렌더링하지 않고 종료 코드로만
// 전달합니다.
#[test]
fn account_completion_maps_refresh_failures_to_nonzero_status() {
    assert_eq!(
        account_exit_code(AccountCompletion::Success),
        ExitCode::SUCCESS
    );
    assert_eq!(
        account_exit_code(AccountCompletion::RefreshFailures),
        ExitCode::FAILURE
    );
}

// 부분 refresh 실패는 결과를 게시한 뒤 일반적인 stderr error 없이 non-zero 상태만 반환합니다.
#[test]
fn account_refresh_failure_is_a_status_only_completion() {
    let mut published = None;
    let result = finish_account_output(
        AccountRunOutput {
            output: "account result\n".to_owned(),
            diagnostics: Vec::new(),
            completion: AccountCompletion::RefreshFailures,
        },
        |output| {
            published = Some(output);
            Ok(())
        },
        |_| Ok(()),
    );

    assert_eq!(published.as_deref(), Some("account result\n"));
    assert!(matches!(result, Ok(AccountCompletion::RefreshFailures)));
}

// stdout 게시 실패는 deferred stderr diagnostic보다 먼저 fatal 오류로 종료합니다.
#[test]
fn account_output_failure_skips_deferred_diagnostics() {
    let result = finish_account_output(
        AccountRunOutput {
            output: "account result\n".to_owned(),
            diagnostics: vec![CliDiagnostic::warning("one warning")],
            completion: AccountCompletion::Success,
        },
        |_| Err(AppError::message("stdout failed")),
        |_| panic!("diagnostics must not be published after stdout failure"),
    );

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("stdout failed"));
}

// stderr diagnostic 게시 실패는 fatal 오류이며 예상된 refresh 실패를 다시 출력하지 않습니다.
#[test]
fn diagnostic_output_failure_is_fatal_after_stdout() {
    let mut published = None;
    let result = finish_account_output(
        AccountRunOutput {
            output: "account result\n".to_owned(),
            diagnostics: vec![CliDiagnostic::warning("one warning")],
            completion: AccountCompletion::RefreshFailures,
        },
        |output| {
            published = Some(output);
            Ok(())
        },
        |_| Err(AppError::message("stderr failed")),
    );

    assert_eq!(published.as_deref(), Some("account result\n"));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("stderr failed"));
}

// 모든 one-shot 결과는 stdout을 먼저 게시한 다음 warning을 게시합니다.
#[test]
fn command_output_publishes_stdout_before_diagnostics() {
    let events = std::cell::RefCell::new(Vec::new());
    let result = finish_command_output(
        "session output\n".to_owned(),
        &[CliDiagnostic::warning("history is read-only")],
        |output| {
            assert_eq!(output, "session output\n");
            events.borrow_mut().push("stdout");
            Ok(())
        },
        |diagnostics| {
            assert_eq!(diagnostics[0].message(), "history is read-only");
            events.borrow_mut().push("stderr");
            Ok(())
        },
    );

    assert!(result.is_ok());
    assert_eq!(events.into_inner(), vec!["stdout", "stderr"]);
}

// stdout 실패 시 결과가 partial이어도 stderr warning을 뒤늦게 쓰지 않습니다.
#[test]
fn command_output_skips_diagnostics_after_stdout_failure() {
    let result = finish_command_output(
        "session output\n".to_owned(),
        &[CliDiagnostic::warning("history is read-only")],
        |_| Err(AppError::message("stdout failed")),
        |_| panic!("diagnostics must not be published after stdout failure"),
    );

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("stdout failed"));
}

// diagnostics sink 실패는 stdout 성공 이후 fatal 오류로 전환됩니다.
#[test]
fn command_output_reports_diagnostics_sink_failure() {
    let mut published = false;
    let result = finish_command_output(
        "session output\n".to_owned(),
        &[CliDiagnostic::warning("history is read-only")],
        |_| {
            published = true;
            Ok(())
        },
        |_| Err(AppError::message("stderr failed")),
    );

    assert!(published);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("stderr failed"));
}

// warning 본문을 쓴 뒤 stderr flush가 실패해도 결과 sink 오류로 보고합니다.
#[test]
fn diagnostics_writer_reports_flush_failure() {
    let error = write_cli_diagnostics_to_and_flush(
        &[CliDiagnostic::warning("history is read-only")],
        &mut FlushFails,
    )
    .unwrap_err();

    assert!(error.to_string().contains("flushing command diagnostics"));
}
